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
use std::io::Read;

use memchr::memchr_iter;

use ctcore::ct_ringbuffer::CtRingBuffer;

/// Create an iterator over all but the last `n` elements of `iter`.
///
/// # Examples
///
/// ```rust,ignore
/// let data = [1, 2, 3, 4, 5];
/// let n = 2;
/// let mut iter = take_all_but(data.iter(), n);
/// assert_eq!(Some(4), iter.next());
/// assert_eq!(Some(5), iter.next());
/// assert_eq!(None, iter.next());
/// ```
pub fn take_all_but<I: Iterator>(iter: I, n: usize) -> TakeAllBut<I> {
    TakeAllBut::new(iter, n)
}

/// An iterator that only iterates over the last elements of another iterator.
pub struct TakeAllBut<I: Iterator> {
    iter: I,
    buf: CtRingBuffer<<I as Iterator>::Item>,
}

impl<I: Iterator> TakeAllBut<I> {
    pub fn new(mut iter: I, n: usize) -> Self {
        // Create a new ring buffer and fill it up.
        //
        // If there are fewer than `n` elements in `iter`, then we
        // exhaust the iterator so that whenever `TakeAllBut::next()` is
        // called, it will return `None`, as expected.
        let mut buf = CtRingBuffer::new(n);
        for _ in 0..n {
            let value = match iter.next() {
                None => {
                    break;
                }
                Some(x) => x,
            };
            buf.push_back(value);
        }
        Self { iter, buf }
    }
}

impl<I: Iterator> Iterator for TakeAllBut<I>
where
    I: Iterator,
{
    type Item = <I as Iterator>::Item;

    fn next(&mut self) -> Option<<I as Iterator>::Item> {
        match self.iter.next() {
            Some(value) => self.buf.push_back(value),
            None => None,
        }
    }
}

/// Like `std::io::Take`, but for lines instead of bytes.
///
/// This struct is generally created by calling [`take_lines`] on a
/// reader. Please see the documentation of [`take_lines`] for more
/// details.
pub struct TakeLines<T> {
    inner: T,
    limit: u64,
    separator: u8,
}

impl<T: Read> Read for TakeLines<T> {
    /// Read bytes from a buffer up to the requested number of lines.
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.limit == 0 {
            return Ok(0);
        }
        match self.inner.read(buf) {
            Ok(0) => Ok(0),
            Ok(n) => {
                for i in memchr_iter(self.separator, &buf[..n]) {
                    self.limit -= 1;
                    if self.limit == 0 {
                        return Ok(i + 1);
                    }
                }
                Ok(n)
            }
            Err(e) => Err(e),
        }
    }
}

/// Create an adaptor that will read at most `limit` lines from a given reader.
///
/// This function returns a new instance of `Read` that will read at
/// most `limit` lines, after which it will always return EOF
/// (`Ok(0)`).
///
/// The `separator` defines the character to interpret as the line
/// ending. For the usual notion of "line", set this to `b'\n'`.
pub fn take_lines<R>(reader: R, limit: u64, separator: u8) -> TakeLines<R> {
    TakeLines {
        inner: reader,
        limit,
        separator,
    }
}

