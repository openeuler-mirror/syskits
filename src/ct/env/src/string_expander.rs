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
use std::{
    ffi::{OsStr, OsString},
    mem,
    ops::Deref,
};

use crate::{
    native_int_str::{NativeCharInt, NativeIntStr, to_native_int_representation},
    string_parser::{Chunk, Error, StringParser},
};

/// `StringExpander` 类为解析和收集单词提供了便利。
///
/// 它管理一个自动填充的"输出"缓冲区。
/// 它提供了"skip_one"和"take_one"方法，专注于使用ASCII分隔符工作。
/// 因此，它们会一次性跳过或获取所有连续的非ASCII字符序列。
pub struct StringExpander<'a> {
    parser: StringParser<'a>,
    pub(crate) output: Vec<NativeCharInt>,
}

impl<'a> StringExpander<'a> {
    /// 创建一个新的 `StringExpander` 实例。
    ///
    /// # 参数
    /// `input` - 一个 `NativeIntStr` 类型的字符串引用，作为解析的输入。
    ///
    /// # 返回值
    /// 返回一个初始化的 `StringExpander` 实例。
    pub fn new(input: &'a NativeIntStr) -> Self {
        Self {
            parser: StringParser::new(input),
            output: Vec::default(),
        }
    }

    /// 在指定位置创建一个新的 `StringExpander` 实例。
    ///
    /// # 参数
    /// `input` - 一个 `NativeIntStr` 类型的字符串引用，作为解析的输入。
    /// `pos` - 开始解析的位置索引。
    ///
    /// # 返回值
    /// 返回一个在指定位置开始的 `StringExpander` 实例。
    pub fn new_at(input: &'a NativeIntStr, pos: usize) -> Self {
        Self {
            parser: StringParser::new_at(input, pos),
            output: Vec::default(),
        }
    }

    /// 获取解析器的只读引用。
    ///
    /// # 返回值
    /// 返回一个 `StringParser` 的只读引用。
    pub fn get_parser(&self) -> &StringParser<'a> {
        &self.parser
    }

    /// 获取解析器的可变引用。
    ///
    /// # 返回值
    /// 返回一个可变的 `StringParser` 引用。
    pub fn get_parser_mut(&mut self) -> &mut StringParser<'a> {
        &mut self.parser
    }

    /// 浏览下一个字符，不消耗它。
    ///
    /// # 返回值
    /// 成功时返回下一个字符，失败时返回错误。
    pub fn peek(&self) -> Result<char, Error> {
        self.parser.peek()
    }

    /// 跳过一个字符，如果是ASCII字符则跳过，否则跳过整个非ASCII字符序列。
    ///
    /// # 返回值
    /// 成功时返回空结果，失败时返回错误。
    pub fn skip_one(&mut self) -> Result<(), Error> {
        self.get_parser_mut().consume_one_ascii_or_all_non_ascii()?;
        Ok(())
    }

    /// 获取当前浏览位置。
    ///
    /// # 返回值
    /// 返回当前浏览位置的索引。
    pub fn get_peek_position(&self) -> usize {
        self.get_parser().get_peek_position()
    }

    /// 消费一个字符，如果是ASCII字符则获取，否则获取整个非ASCII字符序列。
    ///
    /// # 返回值
    /// 成功时返回空结果，失败时返回错误。
    pub fn take_one(&mut self) -> Result<(), Error> {
        let chunks = self.parser.consume_one_ascii_or_all_non_ascii()?;
        for chunk in chunks {
            match chunk {
                Chunk::InvalidEncoding(invalid) => self.output.extend(invalid),
                Chunk::ValidSingleIntChar((_c, ni)) => self.output.push(ni),
            }
        }
        Ok(())
    }

    /// 向输出缓冲区添加一个字符。
    ///
    /// # 参数
    /// `c` - 需要添加到输出缓冲区的字符。
    pub fn put_one_char(&mut self, c: char) {
        let os_str = OsString::from(c.to_string());
        self.put_string(os_str);
    }

    /// 向输出缓冲区添加一个字符串。
    ///
    /// # 参数
    /// `os_str` - 需要添加到输出缓冲区的 `OsStr` 类型的字符串。
    pub fn put_string<S: AsRef<OsStr>>(&mut self, os_str: S) {
        let native = to_native_int_representation(os_str.as_ref());
        self.output.extend(native.deref());
    }

    /// 向输出缓冲区添加一个 `NativeIntStr` 类型的字符串。
    ///
    /// # 参数
    /// `n_str` - 需要添加到输出缓冲区的 `NativeIntStr` 类型的字符串引用。
    pub fn put_native_string(&mut self, n_str: &NativeIntStr) {
        self.output.extend(n_str);
    }

    /// 获取收集的输出并重置输出缓冲区。
    ///
    /// # 返回值
    /// 返回当前收集的输出，同时清空输出缓冲区。
    pub fn take_collected_output(&mut self) -> Vec<NativeCharInt> {
        mem::take(&mut self.output)
    }
}
