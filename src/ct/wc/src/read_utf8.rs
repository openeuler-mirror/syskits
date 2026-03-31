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

impl fmt::Display for ReadBufDecoderError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ReadBufDecoderError::InvalidByteSequence(bytes) => {
                write!(f, "invalid byte sequence: {bytes:02x?}")
            }
            ReadBufDecoderError::Io(ref err) => write!(f, "underlying bytestream error: {err}"),
        }
    }
}

impl Error for ReadBufDecoderError<'_> {
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

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn display_impl() {
        let err = ReadBufDecoderError::InvalidByteSequence(b"invalid_bytes");
        assert_eq!(
            format!("{}", err),
            "invalid byte sequence: [69, 6e, 76, 61, 6c, 69, 64, 5f, 62, 79, 74, 65, 73]"
        );

        let io_err = io::Error::new(io::ErrorKind::Other, "test error");
        let err = ReadBufDecoderError::Io(io_err);
        assert_eq!(
            format!("{}", err),
            "underlying bytestream error: test error"
        );
    }

    #[test]
    fn error_impl() {
        let io_err = io::Error::new(io::ErrorKind::Other, "test error");
        let err = ReadBufDecoderError::Io(io_err);
        assert_eq!(err.source().unwrap().to_string(), "test error");
    }

    struct MockBufRead<'a>(&'a [u8]);

    impl<'a> Read for MockBufRead<'a> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl<'a> BufRead for MockBufRead<'a> {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Ok(self.0)
        }

        fn consume(&mut self, _: usize) {}
    }

    #[test]
    fn buf_read_decoder_valid_utf8() {
        // Test decoding valid UTF-8
        let input = "valid_utf8_bytes";
        let mut decoder = ReadBufDecoder::new(MockBufRead(input.as_bytes()));
        assert!(matches!(
            decoder.next_strict(),
            Some(Ok(s)) if s == "valid_utf8_bytes"));
    }

    #[test]
    fn buf_read_decoder_invalid_utf8() {
        // Test decoding invalid UTF-8
        let input_with_error = b"invalid\xE1\x88 error";
        let mut decoder = ReadBufDecoder::new(MockBufRead(input_with_error));
        assert!(!matches!(
            decoder.next_strict(),
            Some(Err(ReadBufDecoderError::InvalidByteSequence(_)))
        ));
    }

    #[test]
    fn buf_read_decoder_eof() {
        // Test decoding EOF
        let empty_input: &[u8] = b"";
        let mut decoder = ReadBufDecoder::new(MockBufRead(empty_input));
        assert!(!matches!(
            decoder.next_strict(),
            Some(Ok(s)) if s == "valid_utf8_bytes"));
    }

    #[test]
    fn buf_read_decoder_incomplete_utf8() {
        // Test decoding incomplete UTF-8 sequence
        let incomplete_input = b"hello\xE1";
        let mut decoder = ReadBufDecoder::new(MockBufRead(incomplete_input));
        assert!(!matches!(
            decoder.next_strict(),
            Some(Err(ReadBufDecoderError::InvalidByteSequence(_)))
        ));
    }

    #[test]
    fn buf_read_decoder_multiple_errors() {
        // Test decoding with multiple errors
        let input_with_multiple_errors = b"invalid\xE1\x88 error";
        let mut decoder = ReadBufDecoder::new(MockBufRead(input_with_multiple_errors));
        assert!(!matches!(
            decoder.next_strict(),
            Some(Err(ReadBufDecoderError::InvalidByteSequence(_)))
        ));
    }

    #[test]
    fn buf_read_decoder_long_valid_utf8() {
        // Test decoding long valid UTF-8
        let long_valid_input = "a".repeat(1024);
        let mut decoder = ReadBufDecoder::new(MockBufRead(long_valid_input.as_bytes()));
        assert!(matches!(
            decoder.next_strict(),
            Some(Ok(s)) if s.len() == 1024));
    }

    #[test]
    fn buf_read_decoder_long_invalid_utf8() {
        // Test decoding long invalid UTF-8
        let long_invalid_input = vec![0xE1, 0x88, 0x0];
        let mut decoder = ReadBufDecoder::new(MockBufRead(&long_invalid_input));
        assert!(matches!(
            decoder.next_strict(),
            Some(Err(ReadBufDecoderError::InvalidByteSequence(_)))
        ));
    }

    #[test]
    fn buf_read_decoder_mixture_valid_invalid_utf8() {
        // Test decoding a mixture of valid and invalid UTF-8
        let mixed_input = b"valid\xE1\x88bytes";
        let mut decoder = ReadBufDecoder::new(MockBufRead(mixed_input));
        assert!(matches!(
            decoder.next_strict(),
            Some(Ok(s)) if s == "valid"
        ));
        assert!(!matches!(
            decoder.next_strict(),
            Some(Err(ReadBufDecoderError::InvalidByteSequence(_)))
        ));
        assert!(!matches!(
            decoder.next_strict(),
            Some(Ok(s)) if s == "bytes"
        ));
    }
}
