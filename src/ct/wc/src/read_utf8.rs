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
use std::error::Error;
use std::fmt;
use std::io::{self, BufRead};

use crate::utf8::Utf8Incomplete;

/// 封装一个 `std::io::BufRead` 缓冲字节流，并解码为 UTF-8 格式。
#[derive(Debug)]
pub struct ReadBufDecoder<B: BufRead> {
    buf_read: B,
    bytes_consumed: usize,
    incomplete: Utf8Incomplete,
}

#[derive(Debug)]
pub enum ReadBufDecoderError<'a> {
    /// 代表字节流中的一个 UTF-8 错误。
    ///
    /// 在有损解码中，每一个这样的错误都应该用 U+FFFD 代替。
    /// (请参阅 `BufReadDecoder::next_lossy` 和 `BufReadDecoderError::lossy`)
    InvalidByteSequence(&'a [u8]),

    /// 来自底层字节流的输入/输出错误
    Io(io::Error),
}

impl<'a> fmt::Display for ReadBufDecoderError<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ReadBufDecoderError::InvalidByteSequence(bytes) => {
                write!(f, "invalid byte sequence: {bytes:02x?}")
            }
            ReadBufDecoderError::Io(ref err) => write!(f, "underlying bytestream error: {err}"),
        }
    }
}

impl<'a> Error for ReadBufDecoderError<'a> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match *self {
            ReadBufDecoderError::InvalidByteSequence(_) => None,
            ReadBufDecoderError::Io(ref err) => Some(err),
        }
    }
}

impl<B: BufRead> ReadBufDecoder<B> {
    pub fn new(buf_read: B) -> Self {
        Self {
            buf_read,
            bytes_consumed: 0,
            incomplete: Utf8Incomplete::empty(),
        }
    }

    /// 解码并消耗下一大段 UTF-8 输入。
    ///
    /// 该方法会被反复调用，直到返回 `None`为止表示底层字节流的 EOF。
    /// 此方法类似于 `迭代器::next`、
    /// 除了解码块借用了解码器（~迭代器）
    /// 所以在下一个块开始解码之前，它们需要被处理或复制。
    #[allow(clippy::cognitive_complexity)]
    pub fn next_strict(&mut self) -> Option<Result<&str, ReadBufDecoderError>> {
        enum BytesSource {
            BufRead(usize),
            Incomplete,
        }
        macro_rules! try_io {
            ($io_result: expr) => {
                match $io_result {
                    Ok(value) => value,
                    Err(error) => return Some(Err(ReadBufDecoderError::Io(error))),
                }
            };
        }
        let (source, result) = loop {
            if self.bytes_consumed > 0 {
                self.buf_read.consume(self.bytes_consumed);
                self.bytes_consumed = 0;
            }
            let buf = try_io!(self.buf_read.fill_buf());

            // Force loop iteration to go through an explicit `continue`
            enum Unreachable {}
            let _: Unreachable = if self.incomplete.is_empty() {
                if buf.is_empty() {
                    return None; // EOF
                }
                match std::str::from_utf8(buf) {
                    Ok(_) => break (BytesSource::BufRead(buf.len()), Ok(())),
                    Err(error) => {
                        let valid_up_to = error.valid_up_to();
                        if valid_up_to > 0 {
                            break (BytesSource::BufRead(valid_up_to), Ok(()));
                        }
                        match error.error_len() {
                            Some(invalid_sequence_length) => {
                                break (BytesSource::BufRead(invalid_sequence_length), Err(()));
                            }
                            None => {
                                self.bytes_consumed = buf.len();
                                self.incomplete = Utf8Incomplete::new(buf);
                                // need more input bytes
                                continue;
                            }
                        }
                    }
                }
            } else {
                if buf.is_empty() {
                    break (BytesSource::Incomplete, Err(())); // EOF with incomplete code point
                }
                let (consumed, opt_result) = self.incomplete.try_complete_offsets(buf);
                self.bytes_consumed = consumed;
                match opt_result {
                    None => {
                        // need more input bytes
                        continue;
                    }
                    Some(result) => break (BytesSource::Incomplete, result),
                }
            };
        };
        let bytes = match source {
            BytesSource::BufRead(byte_count) => {
                self.bytes_consumed = byte_count;
                let buf = try_io!(self.buf_read.fill_buf());
                &buf[..byte_count]
            }
            BytesSource::Incomplete => self.incomplete.take_buffer(),
        };
        match result {
            Ok(()) => Some(Ok(unsafe { std::str::from_utf8_unchecked(bytes) })),
            Err(()) => Some(Err(ReadBufDecoderError::InvalidByteSequence(bytes))),
        }
    }
}

