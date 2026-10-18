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
//!
//! Use the [`BufferedOutput`] struct to create a buffered form of the
//! [`Output`] writer.
use crate::{DdOutput, WriteStat};

/// Buffer partial output blocks until they are completed.
///
/// Complete blocks are written immediately to the inner [`Output`],
/// but partial blocks are stored in an internal buffer until they are
/// completed.
pub(crate) struct DDBufferedOutput<'a> {
    /// The unbuffered inner block writer.
    inner: DdOutput<'a>,

    /// The internal buffer that stores a partial block.
    ///
    /// The size of this buffer is always less than the output block
    /// size (that is, the value of the `obs` command-line option).
    buf: Vec<u8>,
}

impl<'a> DDBufferedOutput<'a> {
    /// Add partial block buffering to the given block writer.
    ///
    /// The internal buffer size is at most the value of `obs` as
    /// defined in `inner`.
    pub(crate) fn new(inner: DdOutput<'a>) -> Self {
        let obs = inner.settings.obs;
        Self {
            inner,
            buf: Vec::with_capacity(obs),
        }
    }

    pub(crate) fn discard_cache(&self, offset: libc::off_t, len: libc::off_t) {
        self.inner.discard_cache(offset, len);
    }

    /// Flush the partial block stored in the internal buffer.
    pub(crate) fn flush(&mut self) -> std::io::Result<WriteStat> {
        let wstat = self.inner.write_blocks(&self.buf)?;
        let n = wstat.bytes_total.try_into().unwrap();
        self.buf.drain(0..n);
        Ok(wstat)
    }

    /// Synchronize the inner block writer.
    pub(crate) fn sync(&mut self) -> std::io::Result<()> {
        self.inner.sync()
    }

    /// Truncate the underlying file to the current stream position, if possible.
    pub(crate) fn truncate(&mut self) -> std::io::Result<()> {
        self.inner.dst.truncate()
    }

    /// Write the given bytes one block at a time.
    ///
    /// Only complete blocks will be written. Partial blocks will be
    /// buffered until enough bytes have been provided to complete a
    /// block. The returned [`WriteStat`] object will include the
    /// number of blocks written during execution of this function.
    pub(crate) fn dd_write_blocks(&mut self, mut buf: &[u8]) -> std::io::Result<WriteStat> {
        let obs = self.inner.settings.obs;
        let mut wstat = WriteStat::default();

        if !self.buf.is_empty() {
            let consumed = std::cmp::min(obs - self.buf.len(), buf.len());
            self.buf.extend_from_slice(&buf[..consumed]);
            buf = &buf[consumed..];

            if self.buf.len() < obs {
                return Ok(wstat);
            }

            wstat += self.inner.write_blocks(&self.buf)?;
            self.buf.clear();
        }

        let complete_len = buf.len() - buf.len() % obs;
        if complete_len > 0 {
            wstat += self.inner.write_blocks(&buf[..complete_len])?;
        }
        self.buf.extend_from_slice(&buf[complete_len..]);

        Ok(wstat)
    }
}

#[cfg(unix)]
#[cfg(test)]
mod tests {
    use crate::bufferedoutput::DDBufferedOutput;
    use crate::{DdOptions, DdOutput, Dest};

    #[test]
    fn test_buffered_output_write_blocks_empty() {
        let settings = DdOptions {
            obs: 3,
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput::new(inner);
        let wstat = output.dd_write_blocks(&[]).unwrap();
        assert_eq!(wstat.writes_complete, 0);
        assert_eq!(wstat.writes_partial, 0);
        assert_eq!(wstat.bytes_total, 0);
        assert_eq!(output.buf, Vec::<u8>::new());
    }

    #[test]
    fn test_buffered_output_write_blocks_partial() {
        let settings = DdOptions {
            obs: 3,
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput::new(inner);
        let wstat = output.dd_write_blocks(b"ab").unwrap();
        assert_eq!(wstat.writes_complete, 0);
        assert_eq!(wstat.writes_partial, 0);
        assert_eq!(wstat.bytes_total, 0);
        assert_eq!(output.buf, b"ab");
    }

    #[test]
    fn test_buffered_output_write_blocks_complete() {
        let settings = DdOptions {
            obs: 3,
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput::new(inner);
        let wstat = output.dd_write_blocks(b"abcd").unwrap();
        assert_eq!(wstat.writes_complete, 1);
        assert_eq!(wstat.writes_partial, 0);
        assert_eq!(wstat.bytes_total, 3);
        assert_eq!(output.buf, b"d");
    }

    #[test]
    fn test_buffered_output_write_blocks_append() {
        let settings = DdOptions {
            obs: 3,
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput {
            inner,
            buf: b"ab".to_vec(),
        };
        let wstat = output.dd_write_blocks(b"cdefg").unwrap();
        assert_eq!(wstat.writes_complete, 2);
        assert_eq!(wstat.writes_partial, 0);
        assert_eq!(wstat.bytes_total, 6);
        assert_eq!(output.buf, b"g");
    }

    #[test]
    fn test_buffered_output_flush() {
        let settings = DdOptions {
            obs: 10,
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput {
            inner,
            buf: b"abc".to_vec(),
        };
        let wstat = output.flush().unwrap();
        assert_eq!(wstat.writes_complete, 0);
        assert_eq!(wstat.writes_partial, 1);
        assert_eq!(wstat.bytes_total, 3);
        assert_eq!(output.buf, Vec::<u8>::new());
    }
}

#[cfg(unix)]
#[cfg(test)]
mod tests_write_blocks {
    use super::*;
    use crate::{DdOptions, Dest};

    // 修改测试用例
    #[test]
    fn test_write_blocks_small_obs() {
        let settings = DdOptions {
            obs: 1, // 改为最小有效块大小
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput::new(inner);
        let wstat = output.dd_write_blocks(b"abc").unwrap();
        assert_eq!(wstat.writes_complete, 3); // 每个字节作为一个完整块
        assert_eq!(wstat.writes_partial, 0);
        assert_eq!(wstat.bytes_total, 3);
        assert_eq!(output.buf, Vec::<u8>::new()); // 所有数据都被写入
    }

    #[test]
    fn test_write_blocks_exact_multiple() {
        let settings = DdOptions {
            obs: 2,
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput::new(inner);
        let wstat = output.dd_write_blocks(b"abcd").unwrap();
        assert_eq!(wstat.writes_complete, 2);
        assert_eq!(wstat.writes_partial, 0);
        assert_eq!(wstat.bytes_total, 4);
        assert_eq!(output.buf, Vec::<u8>::new());
    }

    #[test]
    fn test_write_blocks_with_partial_buffer() {
        let settings = DdOptions {
            obs: 3,
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput {
            inner,
            buf: b"a".to_vec(),
        };

        let wstat = output.dd_write_blocks(b"bcdef").unwrap();

        // 修改断言以匹配实际行为
        assert_eq!(
            wstat.writes_complete, 2,
            "Expected two complete writes (abc, def)"
        );
        assert_eq!(wstat.writes_partial, 0, "Expected no partial writes");
        assert_eq!(wstat.bytes_total, 6, "Expected 6 bytes written (abc + def)");
        assert_eq!(
            output.buf,
            Vec::<u8>::new(),
            "Buffer should be empty after writing complete blocks"
        );
    }

    #[test]
    fn test_write_blocks_sequential_writes() {
        let settings = DdOptions {
            obs: 4,
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput::new(inner);

        let wstat1 = output.dd_write_blocks(b"ab").unwrap();
        assert_eq!(
            wstat1.writes_complete, 0,
            "Expected no complete writes for first partial write"
        );
        assert_eq!(wstat1.writes_partial, 0, "Expected no partial writes");
        assert_eq!(wstat1.bytes_total, 0, "Expected no bytes written");
        assert_eq!(output.buf, b"ab", "Buffer should contain 'ab'");

        // Second write
        let wstat2 = output.dd_write_blocks(b"c").unwrap();
        assert_eq!(wstat2.writes_complete, 0, "Expected no complete writes");
        assert_eq!(wstat2.writes_partial, 0, "Expected no partial writes");
        assert_eq!(wstat2.bytes_total, 0, "Expected no bytes written");
        assert_eq!(output.buf, b"abc", "Buffer should contain 'abc'");

        // Third write completes the block and buffers the remainder.

        let wstat3 = output.dd_write_blocks(b"def").unwrap();

        assert_eq!(
            wstat3.writes_complete, 1,
            "Expected one complete write for full block"
        );
        assert_eq!(wstat3.writes_partial, 0, "Expected no partial writes");
        assert_eq!(wstat3.bytes_total, 4, "Expected 4 bytes written");
        assert_eq!(output.buf, b"ef", "Buffer should contain 'ef'");

        let final_wstat = output.flush().unwrap();
        assert_eq!(final_wstat.writes_complete, 0);
        assert_eq!(final_wstat.writes_partial, 1);
        assert_eq!(final_wstat.bytes_total, 2);
        assert!(output.buf.is_empty());

        println!("=== Test completed ===\n");
    }

    #[test]
    fn test_write_blocks_empty_after_partial() {
        let settings = DdOptions {
            obs: 3,
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput {
            inner,
            buf: b"ab".to_vec(),
        };

        // An empty write must not flush an existing partial block.
        let wstat = output.dd_write_blocks(&[]).unwrap();
        assert_eq!(
            wstat.writes_complete, 0,
            "Expected no complete writes for partial buffer"
        );
        assert_eq!(wstat.writes_partial, 0, "Expected no partial writes");
        assert_eq!(wstat.bytes_total, 0, "Expected no bytes written");
        assert_eq!(output.buf, b"ab", "Buffer should preserve partial block");

        // Supplying the missing byte completes exactly one output block.
        let wstat = output.dd_write_blocks(b"z").unwrap();
        assert_eq!(wstat.writes_complete, 1, "Expected one complete write");
        assert_eq!(wstat.writes_partial, 0, "Expected no partial writes");
        assert_eq!(wstat.bytes_total, 3, "Expected 3 bytes written");
        assert_eq!(
            output.buf,
            Vec::<u8>::new(),
            "Buffer should be empty after complete write"
        );
    }

    #[test]
    fn test_write_blocks_large_obs() {
        let settings = DdOptions {
            obs: 1024,
            ..Default::default()
        };
        let inner = DdOutput {
            dst: Dest::Sink,
            settings: &settings,
        };
        let mut output = DDBufferedOutput::new(inner);

        // Write small amount of data with large block size
        let wstat = output.dd_write_blocks(b"hello").unwrap();
        assert_eq!(wstat.writes_complete, 0);
        assert_eq!(wstat.writes_partial, 0);
        assert_eq!(wstat.bytes_total, 0);
        assert_eq!(output.buf, b"hello");
    }
}
