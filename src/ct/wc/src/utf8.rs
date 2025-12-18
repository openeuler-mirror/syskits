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

use std::cmp;
use std::str;

/// 增量、零拷贝 UTF-8 解码，带错误处理功能
#[derive(Debug, Copy, Clone)]
pub struct Utf8Incomplete {
    pub buffer: [u8; 4],
    pub buffer_len: u8,
}

impl Utf8Incomplete {
    pub fn empty() -> Self {
        Self {
            buffer: [0, 0, 0, 0],
            buffer_len: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buffer_len == 0
    }

    pub fn new(bytes: &[u8]) -> Self {
        let mut buffer = [0, 0, 0, 0];
        let len = bytes.len();
        buffer[..len].copy_from_slice(bytes);
        Self {
            buffer,
            buffer_len: len as u8,
        }
    }

    pub(crate) fn take_buffer(&mut self) -> &[u8] {
        let len = self.buffer_len as usize;
        self.buffer_len = 0;
        &self.buffer[..len]
    }

    /// (consumed_from_input, None): 输入不足
    /// (consumed_from_input, Some(Err(()))): 缓冲区内的错误字节数
    /// (consumed_from_input, Some(Ok(()))): 缓冲区内的 UTF-8 字符串
    pub(crate) fn try_complete_offsets(&mut self, input: &[u8]) -> (usize, Option<Result<(), ()>>) {
        let initial_buffer_size = self.buffer_len as usize;
        let copied_from_input_size;
        {
            let unwritten = &mut self.buffer[initial_buffer_size..];
            copied_from_input_size = cmp::min(unwritten.len(), input.len());
            unwritten[..copied_from_input_size].copy_from_slice(&input[..copied_from_input_size]);
        }
        let spliced_buf = &self.buffer[..initial_buffer_size + copied_from_input_size];
        match str::from_utf8(spliced_buf) {
            Ok(_) => {
                self.buffer_len = spliced_buf.len() as u8;
                (copied_from_input_size, Some(Ok(())))
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                match valid_up_to > 0 {
                    true => {
                        let consumed = valid_up_to.checked_sub(initial_buffer_size).unwrap();
                        self.buffer_len = valid_up_to as u8;
                        (consumed, Some(Ok(())))
                    }
                    false => {
                        if let Some(invalid_sequence_length) = error.error_len() {
                            let consumed = invalid_sequence_length
                                .checked_sub(initial_buffer_size)
                                .unwrap();
                            self.buffer_len = invalid_sequence_length as u8;
                            (consumed, Some(Err(())))
                        } else {
                            self.buffer_len = spliced_buf.len() as u8;
                            (copied_from_input_size, None)
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let incomplete = Utf8Incomplete::empty();
        assert_eq!(incomplete.buffer, [0, 0, 0, 0]);
        assert_eq!(incomplete.buffer_len, 0);
    }

    #[test]
    fn test_new() {
        let incomplete = Utf8Incomplete::new(b"abcd");
        assert_eq!(incomplete.buffer, [97, 98, 99, 100]);
        assert_eq!(incomplete.buffer_len, 4);
    }

    #[test]
    fn test_take_buffer() {
        let mut incomplete = Utf8Incomplete::new(b"abcd");
        let buffer = incomplete.take_buffer();
        assert_eq!(buffer, [97, 98, 99, 100]);
        assert_eq!(incomplete.buffer_len, 0);
    }

    #[test]
    fn test_try_complete_offsets_not_enough_input() {
        let mut incomplete = Utf8Incomplete::new(b"\xc2");
        let input = b"hello";
        let (consumed, result) = incomplete.try_complete_offsets(input);
        assert_eq!(consumed, 0);
        assert_eq!(result, Some(Err(())));
        assert_eq!(incomplete.buffer, [194, 104, 101, 108]);
        assert_eq!(incomplete.buffer_len, 1);
    }

    #[test]
    fn test_try_complete_offsets_invalid_input() {
        let mut incomplete = Utf8Incomplete::new(b"\xc2");
        let input = b"\x80hello";
        let (consumed, result) = incomplete.try_complete_offsets(input);
        assert_eq!(consumed, 3);
        assert_eq!(result, Some(Ok(())));
        assert_eq!(incomplete.buffer, [194, 128, 104, 101]);
        assert_eq!(incomplete.buffer_len, 4);
    }

    #[test]
    fn test_try_complete_offsets_valid_input() {
        let mut incomplete = Utf8Incomplete::new(b"\xc2");
        let input = b"\x80hello";
        let (consumed, result) = incomplete.try_complete_offsets(input);
        assert_eq!(consumed, 3);
        assert_eq!(result, Some(Ok(())));
        assert_eq!(incomplete.buffer, [194, 128, 104, 101]);
        assert_eq!(incomplete.buffer_len, 4);
    }

    #[test]
    fn test_try_complete_offsets_incomplete_input() {
        let mut incomplete = Utf8Incomplete::new(b"\xc2\x80");
        let input = b"hello";
        let (consumed, result) = incomplete.try_complete_offsets(input);
        assert_eq!(consumed, 2);
        assert_eq!(result, Some(Ok(())));
        assert_eq!(incomplete.buffer, [194, 128, 104, 101]);
        assert_eq!(incomplete.buffer_len, 4);
    }

    #[test]
    fn test_try_complete_offsets_complete_utf8() {
        let mut incomplete = Utf8Incomplete::new("hell".as_bytes());
        let input = " world".as_bytes();
        let (consumed, result) = incomplete.try_complete_offsets(input);
        assert_eq!(consumed, 0);
        assert_eq!(result, Some(Ok(())));
        assert_eq!(incomplete.is_empty(), false);
    }

    #[test]
    fn test_try_complete_offsets_incomplete_utf8() {
        let mut incomplete = Utf8Incomplete::new(&[0xC2, 0x80]);
        let input = "hello".as_bytes();
        let (consumed, result) = incomplete.try_complete_offsets(input);
        assert_eq!(consumed, 2);
        assert_eq!(result, Some(Ok(())));
        assert_eq!(incomplete.is_empty(), false);
    }

    #[test]
    fn test_try_complete_offsets_not_enough_input2() {
        let mut incomplete = Utf8Incomplete::new(&[0xC2]); // Incomplete UTF-8 sequence
        let input = "hello".as_bytes();
        let (consumed, result) = incomplete.try_complete_offsets(input);
        assert_eq!(consumed, 0);
        assert_eq!(result, Some(Err(())));
        assert_eq!(incomplete.is_empty(), false);
    }
}