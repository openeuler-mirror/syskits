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
#![forbid(unsafe_code)]

use std::{borrow::Cow, ffi::OsStr};

use crate::native_int_str::{
    NativeCharInt, NativeIntStr, from_native_int_representation, get_char_from_native_int,
    get_single_native_int_value,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub peek_position: usize,
    pub err_type: ErrorType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ErrorType {
    EndOfInput,
    InternalError,
}

/// 提供一个有效的字符或一个无效的字节序列。
///
/// 无效的字节序列不能以任何有意义的方式分割。
/// 因此，它们需要作为一个整体被消耗。
pub enum Chunk<'a> {
    InvalidEncoding(&'a NativeIntStr),
    ValidSingleIntChar((char, NativeCharInt)),
}

/// 这个类使得按字符解析 OsString 更加方便。
///
/// 它还允许捕获中间位置，以便稍后进行分割。
pub struct StringParser<'a> {
    input: &'a NativeIntStr,
    pointer: usize,
    remaining: &'a NativeIntStr,
}

impl<'a> StringParser<'a> {
    /// 创建一个新的 StringParser 实例。
    pub fn new(input: &'a NativeIntStr) -> Self {
        let mut instance = Self {
            input,
            pointer: 0,
            remaining: input,
        };
        instance.set_pointer(0);
        instance
    }

    /// 在指定的位置创建一个新的 StringParser 实例。
    pub fn new_at(input: &'a NativeIntStr, pos: usize) -> Self {
        let mut instance = Self::new(input);
        instance.set_pointer(pos);
        instance
    }

    /// 获取输入的 NativeIntStr。
    pub fn get_input(&self) -> &'a NativeIntStr {
        self.input
    }

    /// 获取当前 peek 位置。
    pub fn get_peek_position(&self) -> usize {
        self.pointer
    }

    /// 在当前位置 peek 一个字符，返回字符或错误。
    pub fn peek(&self) -> Result<char, Error> {
        self.peek_char_at_pointer(self.pointer)
    }

    fn make_err(&self, err_type: ErrorType) -> Error {
        Error {
            peek_position: self.get_peek_position(),
            err_type,
        }
    }

    /// 在指定位置 peek 一个字符，返回字符或错误。
    pub fn peek_char_at_pointer(&self, at_pointer: usize) -> Result<char, Error> {
        let split = self.input.split_at(at_pointer).1;
        if split.is_empty() {
            return Err(self.make_err(ErrorType::EndOfInput));
        }
        if let Some((c, _ni)) = get_char_from_native_int(split[0]) {
            Ok(c)
        } else {
            Ok('\u{FFFD}')
        }
    }

    /// 根据当前位置获取一个 chunk 和其长度，返回 chunk 和长度或错误。
    fn get_chunk_with_length_at(&self, pointer: usize) -> Result<(Chunk<'a>, usize), Error> {
        let (_before, after) = self.input.split_at(pointer);
        if after.is_empty() {
            return Err(self.make_err(ErrorType::EndOfInput));
        }

        if let Some(c_ni) = get_char_from_native_int(after[0]) {
            Ok((Chunk::ValidSingleIntChar(c_ni), 1))
        } else {
            let mut i = 1;
            while i < after.len() {
                if let Some(_c) = get_char_from_native_int(after[i]) {
                    break;
                }
                i += 1;
            }

            let chunk = &after[0..i];
            Ok((Chunk::InvalidEncoding(chunk), chunk.len()))
        }
    }

    /// 在当前位置 peek 一个 chunk，返回 chunk 或 None。
    pub fn peek_chunk(&self) -> Option<Chunk<'a>> {
        self.get_chunk_with_length_at(self.pointer)
            .ok()
            .map(|(chunk, _)| chunk)
    }

    /// 消费当前位置的 chunk，返回 chunk 或错误。
    pub fn consume_chunk(&mut self) -> Result<Chunk<'a>, Error> {
        let (chunk, len) = self.get_chunk_with_length_at(self.pointer)?;
        self.set_pointer(self.pointer + len);
        Ok(chunk)
    }

    /// 消费一个 ASCII 字符或所有非 ASCII 字符，返回 chunk 集合或错误。
    pub fn consume_one_ascii_or_all_non_ascii(&mut self) -> Result<Vec<Chunk<'a>>, Error> {
        let mut result = Vec::<Chunk<'a>>::new();
        loop {
            let data = self.consume_chunk()?;
            let was_ascii = if let Chunk::ValidSingleIntChar((c, _ni)) = &data {
                c.is_ascii()
            } else {
                false
            };
            result.push(data);
            if was_ascii {
                return Ok(result);
            }

            match self.peek_chunk() {
                Some(Chunk::ValidSingleIntChar((c, _ni))) if c.is_ascii() => return Ok(result),
                None => return Ok(result),
                _ => {}
            }
        }
    }

    /// 跳过多指定字节的数量。
    pub fn skip_multiple(&mut self, skip_byte_count: usize) {
        let end_ptr = self.pointer + skip_byte_count;
        self.set_pointer(end_ptr);
    }

    /// 跳过直到遇到指定字符或到达末尾。
    pub fn skip_until_char_or_end(&mut self, c: char) {
        let native_rep = get_single_native_int_value(&c).unwrap();
        let pos = self.remaining.iter().position(|x| *x == native_rep);

        if let Some(pos) = pos {
            self.set_pointer(self.pointer + pos);
        } else {
            self.set_pointer(self.input.len());
        }
    }

    /// 获取指定范围的子字符串。
    pub fn substring(&self, range: &std::ops::Range<usize>) -> &'a NativeIntStr {
        let (_before1, after1) = self.input.split_at(range.start);
        let (middle, _after2) = after1.split_at(range.end - range.start);
        middle
    }

    /// peek 剩余部分的 OsStr。
    pub fn peek_remaining(&self) -> Cow<'a, OsStr> {
        from_native_int_representation(Cow::Borrowed(self.remaining))
    }

    /// 设置新的指针位置。
    pub fn set_pointer(&mut self, new_pointer: usize) {
        self.pointer = new_pointer;
        let (_before, after) = self.input.split_at(self.pointer);
        self.remaining = after;
    }
}
