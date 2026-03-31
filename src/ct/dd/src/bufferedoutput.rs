/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
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
    pub(crate) fn dd_write_blocks(&mut self, buf: &[u8]) -> std::io::Result<WriteStat> {
        // Split the incoming buffer into two parts: the bytes to write
        // and the bytes to buffer for next time.
        //
        // If `buf` does not include enough bytes to form a full block,
        // just buffer the whole thing and write zero blocks.
        let n = self.buf.len() + buf.len();
        let rem = n % self.inner.settings.obs;
        let i = buf.len().saturating_sub(rem);
        let (to_write, to_buffer) = buf.split_at(i);

        // Concatenate the old partial block with the new bytes to form
        // some number of complete blocks.
        self.buf.extend_from_slice(to_write);

        // Write all complete blocks to the inner block writer.
        //
        // For example, if the output block size were 3, the buffered
        // partial block were `b"ab"` and the new incoming bytes were
        // `b"cdefg"`, then we would write blocks `b"abc"` and
        // b`"def"` to the inner block writer.
        let wstat = self.inner.write_blocks(&self.buf)?;

        // Buffer any remaining bytes as a partial block.
        //
        // Continuing the example above, the last byte `b"g"` would be
        // buffered as a partial block until the next call to
        // `write_blocks()`.
        self.buf.clear();
        self.buf.extend_from_slice(to_buffer);

        Ok(wstat)
    }
}

