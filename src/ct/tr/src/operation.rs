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

//! tr 命令的核心转换操作功能实现
//!
//! 本模块实现了以下主要功能：
//! 1. 字符序列的解析与处理 (Sequence 枚举及其实现)
//! 2. 符号转换器接口 (SymbolTranslator trait)
//! 3. 三种核心操作：
//!    - 删除操作 (DeleteOperation)：删除指定字符集中的字符
//!    - 转换操作 (TranslateOperation)：将一个字符集映射到另一个字符集
//!    - 压缩操作 (SqueezeOperation)：压缩重复字符
//! 4. 输入流处理 (translate_input)：处理输入流并应用转换操作

use ctcore::ct_error::CTError;
use nom::{
    IResult,
    branch::alt,
    bytes::complete::{tag, take},
    character::complete::{digit1, one_of},
    combinator::{map, map_opt, peek, recognize, value},
    multi::{many_m_n, many0},
    sequence::{delimited, preceded, separated_pair},
};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt::{Debug, Display},
    io::{BufRead, Write},
};

/// Unicode 字符常量定义
mod unicodes {
    pub static BEL: u8 = 0x7; // 响铃
    pub static BS: u8 = 0x8; // 退格
    pub static HT: u8 = 0x9; // 水平制表符
    pub static LF: u8 = 0xA; // 换行
    pub static VT: u8 = 0xB; // 垂直制表符
    pub static FF: u8 = 0xC; // 换页
    pub static CR: u8 = 0xD; // 回车
    pub static SPACE: u8 = 0x20; // 空格
    pub static SPACES: &[u8] = &[HT, LF, VT, FF, CR, SPACE]; // 所有空白字符
    pub static BLANK: &[u8] = &[SPACE, HT]; // 空格和制表符
}

/// 序列解析错误类型
#[derive(Debug, Clone)]
pub enum BadSequence {
    /// 缺少字符类名称，如 '[::]'
    MissingCharClassName,
    /// 缺少等价类字符，如 '[==]'
    MissingEquivalentClassChar,
    /// SET2 中包含多个重复构造
    MultipleCharRepeatInSet2,
    /// SET1 中包含重复构造
    CharRepeatInSet1,
    /// 无效的重复次数
    InvalidRepeatCount(String),
    /// 当不截断 SET1 时，SET2 为空
    EmptySet2WhenNotTruncatingSet1,
}

impl Display for BadSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingCharClassName => write!(f, "missing character class name '[::]'"),
            Self::MissingEquivalentClassChar => {
                write!(f, "missing equivalence class character '[==]'")
            }
            Self::MultipleCharRepeatInSet2 => {
                write!(f, "only one [c*] repeat construct may appear in string2")
            }
            Self::CharRepeatInSet1 => {
                write!(f, "the [c*] repeat construct may not appear in string1")
            }
            Self::InvalidRepeatCount(count) => {
                write!(f, "invalid repeat count '{count}' in [c*n] construct")
            }
            Self::EmptySet2WhenNotTruncatingSet1 => {
                write!(f, "when not truncating set1, string2 must be non-empty")
            }
        }
    }
}

impl Error for BadSequence {}
impl CTError for BadSequence {}

/// 字符序列类型
#[derive(Debug, Clone, Copy)]
pub enum Sequence {
    /// 单个字符
    Char(u8),
    /// 字符范围，如 'a-z'
    CharRange(u8, u8),
    /// 字符重复（无限次），如 '[a*]'
    CharStar(u8),
    /// 字符重复（指定次数），如 '[a*5]'
    CharRepeat(u8, usize),
    /// 字母数字字符
    Alnum,
    /// 字母字符
    Alpha,
    /// 空白字符
    Blank,
    /// 控制字符
    Control,
    /// 数字字符
    Digit,
    /// 可打印字符（不含空格）
    Graph,
    /// 小写字母
    Lower,
    /// 可打印字符
    Print,
    /// 标点符号
    Punct,
    /// 空白字符（含空格）
    Space,
    /// 大写字母
    Upper,
    /// 十六进制数字
    Xdigit,
}

impl Sequence {
    /// 将序列展开为字符迭代器
    pub fn flatten(&self) -> Box<dyn Iterator<Item = u8>> {
        match self {
            Self::Char(c) => Box::new(std::iter::once(*c)),
            Self::CharRange(l, r) => Box::new(*l..=*r),
            Self::CharStar(c) => Box::new(std::iter::repeat(*c)),
            Self::CharRepeat(c, n) => Box::new(std::iter::repeat(*c).take(*n)),
            Self::Alnum => Box::new((b'0'..=b'9').chain(b'A'..=b'Z').chain(b'a'..=b'z')),
            Self::Alpha => Box::new((b'A'..=b'Z').chain(b'a'..=b'z')),
            Self::Blank => Box::new(unicodes::BLANK.iter().cloned()),
            Self::Control => Box::new((0..=31).chain(std::iter::once(127))),
            Self::Digit => Box::new(b'0'..=b'9'),
            Self::Graph => Box::new(
                (48..=57) // digit
                    .chain(65..=90) // uppercase
                    .chain(97..=122) // lowercase
                    // punctuations
                    .chain(33..=47)
                    .chain(58..=64)
                    .chain(91..=96)
                    .chain(123..=126)
                    .chain(std::iter::once(32)), // space
            ),
            Self::Lower => Box::new(b'a'..=b'z'),
            Self::Print => Box::new(
                (48..=57) // digit
                    .chain(65..=90) // uppercase
                    .chain(97..=122) // lowercase
                    // punctuations
                    .chain(33..=47)
                    .chain(58..=64)
                    .chain(91..=96)
                    .chain(123..=126),
            ),
            Self::Punct => Box::new((33..=47).chain(58..=64).chain(91..=96).chain(123..=126)),
            Self::Space => Box::new(unicodes::SPACES.iter().cloned()),
            Self::Upper => Box::new(b'A'..=b'Z'),
            Self::Xdigit => Box::new((b'0'..=b'9').chain(b'A'..=b'F').chain(b'a'..=b'f')),
        }
    }

    /// 处理字符集，将序列集合展开为字符向量
    fn process_char_set(set: &[Self]) -> Vec<u8> {
        set.iter().flat_map(Self::flatten).collect()
    }

    /// 在序列集合中查找并验证字符星号
    fn find_char_star(set: &[Self]) -> Option<u8> {
        set.iter().find_map(|s| match s {
            Self::CharStar(c) => Some(*c),
            _ => None,
        })
    }

    /// 解析和处理字符集
    ///
    /// # 参数
    /// * `set1_str` - 第一个字符集的字节序列
    /// * `set2_str` - 第二个字符集的字节序列
    /// * `truncate_set1_flag` - 是否需要截断 set1 到 set2 的长度
    ///
    /// # 返回值
    /// 返回处理后的两个字符集，或处理过程中的错误
    pub fn solve_set_characters(
        set1_str: &[u8],
        set2_str: &[u8],
        truncate_set1_flag: bool,
    ) -> Result<(Vec<u8>, Vec<u8>), BadSequence> {
        // Parse and validate set1
        let set1 = Self::from_str(set1_str)?;
        if set1.iter().any(|s| matches!(s, Self::CharStar(_))) {
            return Err(BadSequence::CharRepeatInSet1);
        }

        // Parse and validate set2
        let set2 = Self::from_str(set2_str)?;
        if set2
            .iter()
            .filter(|s| matches!(s, Self::CharStar(_)))
            .count()
            >= 2
        {
            return Err(BadSequence::MultipleCharRepeatInSet2);
        }

        // Process set1
        let mut set1_solved = Self::process_char_set(&set1);

        // Process set2
        let set2_solved = if let Some(char_star) = Self::find_char_star(&set2) {
            let mut result = Vec::new();
            let mut parts = set2.split(|s| matches!(s, Self::CharStar(_)));

            // Add left part if exists
            if let Some(left) = parts.next() {
                result.extend(Self::process_char_set(left));
            }

            // Add repeated characters
            let non_star_len = set2
                .iter()
                .filter(|s| !matches!(s, Self::CharStar(_)))
                .flat_map(Self::flatten)
                .count();
            let repeat_len = set1_solved.len().saturating_sub(non_star_len);
            result.extend(std::iter::repeat(char_star).take(repeat_len));

            // Add right part if exists
            if let Some(right) = parts.next() {
                result.extend(Self::process_char_set(right));
            }

            result
        } else {
            Self::process_char_set(&set2)
        };

        // Apply truncation if needed
        if truncate_set1_flag {
            set1_solved.truncate(set2_solved.len());
        }

        Ok((set1_solved, set2_solved))
    }
}

impl Sequence {
    pub fn from_str(input: &[u8]) -> Result<Vec<Self>, BadSequence> {
        many0(alt((
            Self::parse_char_range,
            Self::parse_char_star,
            Self::parse_char_repeat,
            Self::parse_class,
            Self::parse_char_equal,
            // NOTE: This must be the last one
            map(Self::parse_backslash_or_char, |s| Ok(Self::Char(s))),
        )))(input)
        .map(|(_, r)| r)
        .unwrap()
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
    }

    fn parse_octal(input: &[u8]) -> IResult<&[u8], u8> {
        map_opt(
            preceded(tag("\\"), recognize(many_m_n(1, 3, one_of("01234567")))),
            |out: &[u8]| u8::from_str_radix(std::str::from_utf8(out).expect("boop"), 8).ok(),
        )(input)
    }

    fn parse_backslash(input: &[u8]) -> IResult<&[u8], u8> {
        preceded(tag("\\"), Self::single_char)(input).map(|(l, a)| {
            let c = match a {
                b'a' => unicodes::BEL,
                b'b' => unicodes::BS,
                b'f' => unicodes::FF,
                b'n' => unicodes::LF,
                b'r' => unicodes::CR,
                b't' => unicodes::HT,
                b'v' => unicodes::VT,
                x => x,
            };
            (l, c)
        })
    }

    fn parse_backslash_or_char(input: &[u8]) -> IResult<&[u8], u8> {
        alt((Self::parse_octal, Self::parse_backslash, Self::single_char))(input)
    }

    fn single_char(input: &[u8]) -> IResult<&[u8], u8> {
        take(1usize)(input).map(|(l, a)| (l, a[0]))
    }

    fn parse_char_range(input: &[u8]) -> IResult<&[u8], Result<Self, BadSequence>> {
        separated_pair(
            Self::parse_backslash_or_char,
            tag("-"),
            Self::parse_backslash_or_char,
        )(input)
        .map(|(l, (a, b))| {
            (l, {
                let (start, end) = (u32::from(a), u32::from(b));
                Ok(Self::CharRange(start as u8, end as u8))
            })
        })
    }

    fn parse_char_star(input: &[u8]) -> IResult<&[u8], Result<Self, BadSequence>> {
        delimited(tag("["), Self::parse_backslash_or_char, tag("*]"))(input)
            .map(|(l, a)| (l, Ok(Self::CharStar(a))))
    }

    fn parse_char_repeat(input: &[u8]) -> IResult<&[u8], Result<Self, BadSequence>> {
        delimited(
            tag("["),
            separated_pair(Self::parse_backslash_or_char, tag("*"), digit1),
            tag("]"),
        )(input)
        .map(|(l, (c, cnt_str))| {
            let s = String::from_utf8_lossy(cnt_str);
            let result = if cnt_str.starts_with(b"0") {
                match usize::from_str_radix(&s, 8) {
                    Ok(0) => Ok(Self::CharStar(c)),
                    Ok(count) => Ok(Self::CharRepeat(c, count)),
                    Err(_) => Err(BadSequence::InvalidRepeatCount(s.to_string())),
                }
            } else {
                match s.parse::<usize>() {
                    Ok(0) => Ok(Self::CharStar(c)),
                    Ok(count) => Ok(Self::CharRepeat(c, count)),
                    Err(_) => Err(BadSequence::InvalidRepeatCount(s.to_string())),
                }
            };
            (l, result)
        })
    }

    fn parse_class(input: &[u8]) -> IResult<&[u8], Result<Self, BadSequence>> {
        delimited(
            tag("[:"),
            alt((
                map(
                    alt((
                        value(Self::Alnum, tag("alnum")),
                        value(Self::Alpha, tag("alpha")),
                        value(Self::Blank, tag("blank")),
                        value(Self::Control, tag("cntrl")),
                        value(Self::Digit, tag("digit")),
                        value(Self::Graph, tag("graph")),
                        value(Self::Lower, tag("lower")),
                        value(Self::Print, tag("print")),
                        value(Self::Punct, tag("punct")),
                        value(Self::Space, tag("space")),
                        value(Self::Upper, tag("upper")),
                        value(Self::Xdigit, tag("xdigit")),
                    )),
                    Ok,
                ),
                value(Err(BadSequence::MissingCharClassName), tag("")),
            )),
            tag(":]"),
        )(input)
    }

    fn parse_char_equal(input: &[u8]) -> IResult<&[u8], Result<Self, BadSequence>> {
        delimited(
            tag("[="),
            alt((
                value(
                    Err(BadSequence::MissingEquivalentClassChar),
                    peek(tag("=]")),
                ),
                map(Self::parse_backslash_or_char, |c| Ok(Self::Char(c))),
            )),
            tag("=]"),
        )(input)
    }
}

/// 符号转换器特征
///
/// 定义了字符转换的核心接口，所有具体的转换操作都需要实现此特征
pub trait SymbolTranslator {
    /// 转换单个字符
    ///
    /// # 参数
    /// * `current` - 待转换的字符
    ///
    /// # 返回值
    /// 返回转换后的字符，如果字符应该被删除则返回 None
    fn translate(&mut self, current: u8) -> Option<u8>;

    /// 链接两个转换器
    ///
    /// 创建一个新的转换器，将两个转换器按顺序应用
    fn chain<T>(self, other: T) -> ChainedSymbolTranslator<Self, T>
    where
        Self: Sized,
    {
        ChainedSymbolTranslator {
            stage_a: self,
            stage_b: other,
        }
    }
}

/// 链式转换器，用于组合多个转换操作
pub struct ChainedSymbolTranslator<A, B> {
    stage_a: A,
    stage_b: B,
}

impl<A: SymbolTranslator, B: SymbolTranslator> SymbolTranslator for ChainedSymbolTranslator<A, B> {
    fn translate(&mut self, current: u8) -> Option<u8> {
        self.stage_a
            .translate(current)
            .and_then(|c| self.stage_b.translate(c))
    }

    fn chain<T>(self, other: T) -> ChainedSymbolTranslator<Self, T>
    where
        Self: Sized,
    {
        ChainedSymbolTranslator {
            stage_a: self,
            stage_b: other,
        }
    }
}

/// 删除操作的实现
#[derive(Debug)]
pub struct DeleteOperation {
    /// 要删除的字符集
    set: Vec<u8>,
    /// 是否对字符集取补集
    is_complement_flag: bool,
}

impl DeleteOperation {
    /// 创建新的删除操作
    pub fn new(set: Vec<u8>, complement_flag: bool) -> Self {
        Self {
            set,
            is_complement_flag: complement_flag,
        }
    }
}

impl SymbolTranslator for DeleteOperation {
    fn translate(&mut self, current: u8) -> Option<u8> {
        let found = self.set.iter().any(|sequence| *sequence == current);
        if self.is_complement_flag == found {
            Some(current)
        } else {
            None
        }
    }

    fn chain<T>(self, other: T) -> ChainedSymbolTranslator<Self, T>
    where
        Self: Sized,
    {
        ChainedSymbolTranslator {
            stage_a: self,
            stage_b: other,
        }
    }
}

/// 补集转换操作的实现
#[derive(Debug)]
pub struct TranslateOperationComplement {
    /// 当前迭代位置
    iter: u8,
    /// set2 的迭代位置
    set2_iter: usize,
    /// 第一个字符集
    set1: Vec<u8>,
    /// 第二个字符集
    set2: Vec<u8>,
    /// 字符映射表
    translation_map: HashMap<u8, u8>,
}

impl TranslateOperationComplement {
    /// 创建新的补集转换操作
    fn new(set1: Vec<u8>, set2: Vec<u8>) -> Self {
        Self {
            iter: 0,
            set2_iter: 0,
            set1,
            set2,
            translation_map: HashMap::new(),
        }
    }
}

/// 标准转换操作的实现
#[derive(Debug)]
pub struct TranslateOperationStandard {
    /// 字符映射表
    translation_map: HashMap<u8, u8>,
}

impl TranslateOperationStandard {
    /// 创建新的标准转换操作
    fn new(set1: Vec<u8>, set2: Vec<u8>) -> Result<Self, BadSequence> {
        if let Some(fallback) = set2.last().copied() {
            Ok(Self {
                translation_map: set1
                    .into_iter()
                    .zip(set2.into_iter().chain(std::iter::repeat(fallback)))
                    .collect::<HashMap<_, _>>(),
            })
        } else if set1.is_empty() && set2.is_empty() {
            Ok(Self {
                translation_map: HashMap::new(),
            })
        } else {
            Err(BadSequence::EmptySet2WhenNotTruncatingSet1)
        }
    }
}

/// 转换操作的枚举类型
#[derive(Debug)]
pub enum TranslateOperation {
    /// 标准转换模式
    Standard(TranslateOperationStandard),
    /// 补集转换模式
    Complement(TranslateOperationComplement),
}

impl TranslateOperation {
    /// 查找下一个补集字符
    fn next_complement_char(iter: u8, ignore_list: &[u8]) -> (u8, u8) {
        (iter..)
            .filter(|c| !ignore_list.iter().any(|s| s == c))
            .map(|c| (c + 1, c))
            .next()
            .expect("exhausted all possible characters")
    }
}

impl TranslateOperation {
    /// 创建新的转换操作
    pub fn new(set1: Vec<u8>, set2: Vec<u8>, is_complement: bool) -> Result<Self, BadSequence> {
        if is_complement {
            Ok(Self::Complement(TranslateOperationComplement::new(
                set1, set2,
            )))
        } else {
            Ok(Self::Standard(TranslateOperationStandard::new(set1, set2)?))
        }
    }
}

impl SymbolTranslator for TranslateOperation {
    fn translate(&mut self, current: u8) -> Option<u8> {
        match self {
            Self::Standard(TranslateOperationStandard { translation_map }) => {
                Some(*translation_map.get(&current).unwrap_or(&current))
            }
            Self::Complement(complement_op) => {
                if let Some(c) = complement_op.set1.iter().find(|c| c.eq(&&current)) {
                    Some(*c)
                } else {
                    let value = if let Some(value) = complement_op.set2.get(complement_op.set2_iter)
                    {
                        let (next_iter, next_key) =
                            Self::next_complement_char(complement_op.iter, &complement_op.set1);
                        complement_op.iter = next_iter;
                        complement_op.set2_iter = complement_op.set2_iter.saturating_add(1);
                        complement_op.translation_map.insert(next_key, *value);
                        *value
                    } else {
                        let fallback = *complement_op.set2.last().unwrap_or(&current);
                        complement_op.translation_map.insert(current, fallback);
                        fallback
                    };
                    Some(value)
                }
            }
        }
    }

    fn chain<T>(self, other: T) -> ChainedSymbolTranslator<Self, T>
    where
        Self: Sized,
    {
        ChainedSymbolTranslator {
            stage_a: self,
            stage_b: other,
        }
    }
}

/// 压缩操作的实现
#[derive(Debug, Clone)]
pub struct SqueezeOperation {
    /// 要压缩的字符集
    set1: HashSet<u8>,
    /// 是否对字符集取补集
    is_complement: bool,
    /// 前一个处理的字符
    previous: Option<u8>,
}

impl SqueezeOperation {
    /// 创建新的压缩操作
    pub fn new(set1: Vec<u8>, is_complement: bool) -> Self {
        Self {
            set1: set1.into_iter().collect(),
            is_complement,
            previous: None,
        }
    }
}

impl SymbolTranslator for SqueezeOperation {
    fn translate(&mut self, current: u8) -> Option<u8> {
        if self.is_complement {
            let next = if self.set1.contains(&current) {
                Some(current)
            } else {
                match self.previous {
                    Some(v) => {
                        if v.eq(&current) {
                            None
                        } else {
                            Some(current)
                        }
                    }
                    None => Some(current),
                }
            };
            self.previous = Some(current);
            next
        } else {
            let next = if self.set1.contains(&current) {
                match self.previous {
                    Some(v) if v == current => None,
                    _ => Some(current),
                }
            } else {
                Some(current)
            };
            self.previous = Some(current);
            next
        }
    }

    fn chain<T>(self, other: T) -> ChainedSymbolTranslator<Self, T>
    where
        Self: Sized,
    {
        ChainedSymbolTranslator {
            stage_a: self,
            stage_b: other,
        }
    }
}

/// 处理输入流，应用转换操作
///
/// # 参数
/// * `input` - 输入流
/// * `output` - 输出流
/// * `translator` - 转换器
pub fn translate_input<T, R, W>(input: &mut R, output: &mut W, mut translator: T)
where
    T: SymbolTranslator,
    R: BufRead,
    W: Write,
{
    const BUFFER_SIZE: usize = 8192;
    let mut buf = Vec::with_capacity(BUFFER_SIZE);
    let mut output_buf = Vec::with_capacity(BUFFER_SIZE);

    while let Ok(length) = input.read_until(b'\n', &mut buf) {
        if length == 0 {
            break;
        }

        output_buf.clear();
        output_buf.extend(buf.iter().filter_map(|&c| translator.translate(c)));

        if let Err(e) = output.write_all(&output_buf) {
            eprintln!("Error writing output: {}", e);
            break;
        }

        buf.clear();
    }

    if let Err(e) = output.flush() {
        eprintln!("Error flushing output: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试序列解析相关功能
    mod sequence_tests {
        use super::*;

        #[test]
        fn test_sequence_flatten() {
            // 测试单个字符
            let char_seq = Sequence::Char(b'a');
            assert_eq!(char_seq.flatten().collect::<Vec<_>>(), vec![b'a']);

            // 测试字符范围
            let range_seq = Sequence::CharRange(b'a', b'c');
            assert_eq!(
                range_seq.flatten().collect::<Vec<_>>(),
                vec![b'a', b'b', b'c']
            );

            // 测试字符重复
            let repeat_seq = Sequence::CharRepeat(b'x', 3);
            assert_eq!(
                repeat_seq.flatten().collect::<Vec<_>>(),
                vec![b'x', b'x', b'x']
            );

            // 测试预定义字符类
            let digit_seq = Sequence::Digit;
            assert_eq!(
                digit_seq.flatten().collect::<Vec<_>>(),
                (b'0'..=b'9').collect::<Vec<_>>()
            );

            // 测试空白字符类
            let blank_seq = Sequence::Blank;
            assert_eq!(
                blank_seq.flatten().collect::<Vec<_>>(),
                vec![unicodes::SPACE, unicodes::HT]
            );

            // 测试控制字符类
            let control_seq = Sequence::Control;
            let control_chars: Vec<u8> = (0..=31).chain(std::iter::once(127)).collect();
            assert_eq!(control_seq.flatten().collect::<Vec<_>>(), control_chars);

            // 测试标点符号类
            let punct_seq = Sequence::Punct;
            let punct_chars: Vec<u8> = (33..=47)
                .chain(58..=64)
                .chain(91..=96)
                .chain(123..=126)
                .collect();
            assert_eq!(punct_seq.flatten().collect::<Vec<_>>(), punct_chars);
        }

        #[test]
        fn test_sequence_from_str() {
            // 测试基本字符
            assert!(matches!(
                Sequence::from_str(b"a").unwrap()[0],
                Sequence::Char(b'a')
            ));

            // 测试字符范围
            let range = Sequence::from_str(b"a-z").unwrap();
            assert!(matches!(range[0], Sequence::CharRange(b'a', b'z')));

            // 测试字符类
            assert!(matches!(
                Sequence::from_str(b"[:digit:]").unwrap()[0],
                Sequence::Digit
            ));

            // 测试转义序列
            assert!(matches!(
                Sequence::from_str(b"\\n").unwrap()[0],
                Sequence::Char(0x0A)
            ));
        }

        #[test]
        fn test_sequence_from_str_special_cases() {
            // 测试八进制转义序列
            assert!(matches!(
                Sequence::from_str(b"\\101").unwrap()[0],
                Sequence::Char(65) // 'A' in ASCII
            ));

            // 测试无限重复
            assert!(matches!(
                Sequence::from_str(b"[x*]").unwrap()[0],
                Sequence::CharStar(b'x')
            ));

            // 测试指定次数重复
            assert!(matches!(
                Sequence::from_str(b"[x*5]").unwrap()[0],
                Sequence::CharRepeat(b'x', 5)
            ));

            // 测试等价类
            assert!(matches!(
                Sequence::from_str(b"[=a=]").unwrap()[0],
                Sequence::Char(b'a')
            ));
        }

        #[test]
        fn test_sequence_error_cases() {
            // 测试缺少等价类字符
            assert!(matches!(
                Sequence::from_str(b"[==]").unwrap_err(),
                BadSequence::MissingEquivalentClassChar
            ));

            // 测试 SET1 中的重复构造
            assert!(matches!(
                Sequence::solve_set_characters(b"[x*]", b"a", false).unwrap_err(),
                BadSequence::CharRepeatInSet1
            ));

            // 测试 SET2 中的多个重复构造
            assert!(matches!(
                Sequence::solve_set_characters(b"a", b"[x*][y*]", false).unwrap_err(),
                BadSequence::MultipleCharRepeatInSet2
            ));

            // 测试 SET2 为空但不截断时的错误
            assert!(matches!(
                TranslateOperation::new(vec![b'a'], vec![], false).unwrap_err(),
                BadSequence::EmptySet2WhenNotTruncatingSet1
            ));
        }

        #[test]
        fn test_solve_set_characters() {
            // 测试基本转换
            let (set1, set2) = Sequence::solve_set_characters(b"abc", b"123", false).unwrap();
            assert_eq!(set1, vec![b'a', b'b', b'c']);
            assert_eq!(set2, vec![b'1', b'2', b'3']);

            // 测试截断
            let (set1, set2) = Sequence::solve_set_characters(b"abcd", b"12", true).unwrap();
            assert_eq!(set1, vec![b'a', b'b']);
            assert_eq!(set2, vec![b'1', b'2']);

            // 测试字符重复
            let (set1, set2) = Sequence::solve_set_characters(b"abc", b"1[x*]3", false).unwrap();
            assert_eq!(set1, vec![b'a', b'b', b'c']);
            assert_eq!(set2, vec![b'1', b'x', b'3']);
        }

        #[test]
        fn test_sequence_process_char_set() {
            // 测试空集
            let empty_set: Vec<Sequence> = vec![];
            assert_eq!(Sequence::process_char_set(&empty_set), Vec::<u8>::new());

            // 测试混合集合
            let mixed_set = vec![
                Sequence::Char(b'a'),
                Sequence::CharRange(b'1', b'3'),
                Sequence::CharRepeat(b'x', 2),
            ];
            assert_eq!(
                Sequence::process_char_set(&mixed_set),
                vec![b'a', b'1', b'2', b'3', b'x', b'x']
            );
        }

        #[test]
        fn test_find_char_star() {
            // 测试空集
            let empty_set: Vec<Sequence> = vec![];
            assert_eq!(Sequence::find_char_star(&empty_set), None);

            // 测试无星号的集合
            let no_star_set = vec![Sequence::Char(b'a'), Sequence::CharRange(b'1', b'3')];
            assert_eq!(Sequence::find_char_star(&no_star_set), None);

            // 测试有星号的集合
            let star_set = vec![
                Sequence::Char(b'a'),
                Sequence::CharStar(b'x'),
                Sequence::CharRange(b'1', b'3'),
            ];
            assert_eq!(Sequence::find_char_star(&star_set), Some(b'x'));

            // 测试多个星号的集合（应返回第一个）
            let multi_star_set = vec![
                Sequence::CharStar(b'x'),
                Sequence::Char(b'a'),
                Sequence::CharStar(b'y'),
            ];
            assert_eq!(Sequence::find_char_star(&multi_star_set), Some(b'x'));
        }

        #[test]
        fn test_solve_set_characters_edge_cases() {
            // 测试两个空集
            let (set1, set2) = Sequence::solve_set_characters(b"", b"", false).unwrap();
            assert_eq!(set1, Vec::<u8>::new());
            assert_eq!(set2, Vec::<u8>::new());

            // 测试 set1 比 set2 短
            let (set1, set2) = Sequence::solve_set_characters(b"a", b"123", false).unwrap();
            assert_eq!(set1, vec![b'a']);
            assert_eq!(set2, vec![b'1', b'2', b'3']);

            // 测试 set2 中的字符星号填充
            let (set1, set2) = Sequence::solve_set_characters(b"abcde", b"1[x*]5", false).unwrap();
            assert_eq!(set1, vec![b'a', b'b', b'c', b'd', b'e']);
            assert_eq!(set2, vec![b'1', b'x', b'x', b'x', b'5']);

            // 测试 set2 中的字符星号在开头
            let (set1, set2) = Sequence::solve_set_characters(b"abc", b"[x*]yz", false).unwrap();
            assert_eq!(set1, vec![b'a', b'b', b'c']);
            assert_eq!(set2, vec![b'x', b'y', b'z']);

            // 测试 set2 中的字符星号在结尾
            let (set1, set2) = Sequence::solve_set_characters(b"abc", b"12[x*]", false).unwrap();
            assert_eq!(set1, vec![b'a', b'b', b'c']);
            assert_eq!(set2, vec![b'1', b'2', b'x']);
        }

        #[test]
        fn test_parse_octal() {
            // 测试有效的八进制转义序列
            let (_, result) = Sequence::parse_octal(b"\\101").unwrap();
            assert_eq!(result, 65); // 'A' in ASCII

            let (_, result) = Sequence::parse_octal(b"\\7").unwrap();
            assert_eq!(result, 7);

            let (_, result) = Sequence::parse_octal(b"\\12").unwrap();
            assert_eq!(result, 10);
        }

        #[test]
        fn test_parse_backslash() {
            // 测试特殊转义字符
            let (_, result) = Sequence::parse_backslash(b"\\a").unwrap();
            assert_eq!(result, unicodes::BEL);

            let (_, result) = Sequence::parse_backslash(b"\\t").unwrap();
            assert_eq!(result, unicodes::HT);

            let (_, result) = Sequence::parse_backslash(b"\\n").unwrap();
            assert_eq!(result, unicodes::LF);

            // 测试普通字符的转义
            let (_, result) = Sequence::parse_backslash(b"\\x").unwrap();
            assert_eq!(result, b'x');
        }
    }

    /// 测试删除操作相关功能
    mod delete_operation_tests {
        use super::*;

        #[test]
        fn test_delete_operation() {
            // 测试基本删除
            let mut op = DeleteOperation::new(vec![b'a', b'e', b'i', b'o', b'u'], false);
            assert_eq!(op.translate(b'a'), None);
            assert_eq!(op.translate(b'b'), Some(b'b'));
            assert_eq!(op.translate(b'e'), None);

            // 测试补集删除
            let mut op = DeleteOperation::new(vec![b'a', b'e'], true);
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'b'), None);
            assert_eq!(op.translate(b'e'), Some(b'e'));
        }

        #[test]
        fn test_delete_operation_complex() {
            // 测试空集删除
            let mut op = DeleteOperation::new(vec![], false);
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'\n'), Some(b'\n'));

            // 测试全集删除（补集模式）
            let mut op = DeleteOperation::new(vec![], true);
            assert_eq!(op.translate(b'a'), None);
            assert_eq!(op.translate(b'\n'), None);

            // 测试特殊字符删除
            let mut op = DeleteOperation::new(vec![b'\n', b'\t', b' '], false);
            assert_eq!(op.translate(b'\n'), None);
            assert_eq!(op.translate(b'\t'), None);
            assert_eq!(op.translate(b' '), None);
            assert_eq!(op.translate(b'x'), Some(b'x'));
        }

        #[test]
        fn test_delete_operation_with_special_chars() {
            // 测试删除控制字符
            let mut op =
                DeleteOperation::new(vec![unicodes::HT, unicodes::LF, unicodes::CR], false);
            assert_eq!(op.translate(unicodes::HT), None);
            assert_eq!(op.translate(unicodes::LF), None);
            assert_eq!(op.translate(unicodes::CR), None);
            assert_eq!(op.translate(b'a'), Some(b'a'));

            // 测试删除ASCII范围外的字符（假设我们处理的是u8）
            let mut op = DeleteOperation::new(vec![127, 128, 255], false);
            assert_eq!(op.translate(127), None);
            assert_eq!(op.translate(128), None);
            assert_eq!(op.translate(255), None);
            assert_eq!(op.translate(b'a'), Some(b'a'));
        }

        #[test]
        fn test_delete_operation_with_large_set() {
            // 创建一个大的字符集
            let large_set: Vec<u8> = (0..128).collect();
            let mut op = DeleteOperation::new(large_set, false);

            // 测试删除范围内的字符
            for i in 0..128 {
                assert_eq!(op.translate(i), None);
            }

            // 测试范围外的字符
            for i in 128..=255 {
                assert_eq!(op.translate(i), Some(i));
            }
        }
    }

    /// 测试转换操作相关功能
    mod translate_operation_tests {
        use super::*;

        #[test]
        fn test_standard_translation() {
            // 测试基本转换
            let mut op =
                TranslateOperation::new(vec![b'a', b'b', b'c'], vec![b'1', b'2', b'3'], false)
                    .unwrap();
            assert_eq!(op.translate(b'a'), Some(b'1'));
            assert_eq!(op.translate(b'b'), Some(b'2'));
            assert_eq!(op.translate(b'c'), Some(b'3'));
            assert_eq!(op.translate(b'x'), Some(b'x')); // 未映射字符保持不变
        }

        #[test]
        fn test_complement_translation() {
            // 测试补集转换
            let mut op = TranslateOperation::new(vec![b'a', b'b'], vec![b'1', b'2'], true).unwrap();
            assert_eq!(op.translate(b'a'), Some(b'a')); // 在集合中的字符保持不变
            assert_eq!(op.translate(b'x'), Some(b'1')); // 不在集合中的字符被映射
            assert_eq!(op.translate(b'y'), Some(b'2')); // 不在集合中的下一个字符被映射
        }

        #[test]
        fn test_translate_edge_cases() {
            // 测试空集转换
            let mut op = TranslateOperation::new(vec![], vec![], false).unwrap();
            assert_eq!(op.translate(b'a'), Some(b'a'));

            // 测试单字符到多字符的映射
            let mut op =
                TranslateOperation::new(vec![b'a'], vec![b'1', b'2', b'3'], false).unwrap();
            assert_eq!(op.translate(b'a'), Some(b'1'));
            assert_eq!(op.translate(b'b'), Some(b'b'));

            // 测试补集模式下的边界情况
            let mut op = TranslateOperation::new(vec![b'a'], vec![b'1'], true).unwrap();
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'b'), Some(b'1'));
        }

        #[test]
        fn test_translate_error_cases() {
            // 测试 SET2 为空但不截断时的错误
            assert!(matches!(
                TranslateOperation::new(vec![b'a'], vec![], false).unwrap_err(),
                BadSequence::EmptySet2WhenNotTruncatingSet1
            ));
        }

        #[test]
        fn test_translate_operation_with_special_chars() {
            // 测试转换控制字符
            let mut op = TranslateOperation::new(
                vec![unicodes::HT, unicodes::LF, unicodes::CR],
                vec![b'1', b'2', b'3'],
                false,
            )
            .unwrap();
            assert_eq!(op.translate(unicodes::HT), Some(b'1'));
            assert_eq!(op.translate(unicodes::LF), Some(b'2'));
            assert_eq!(op.translate(unicodes::CR), Some(b'3'));
            assert_eq!(op.translate(b'a'), Some(b'a'));

            // 测试转换ASCII范围外的字符
            let mut op =
                TranslateOperation::new(vec![127, 128, 255], vec![b'1', b'2', b'3'], false)
                    .unwrap();
            assert_eq!(op.translate(127), Some(b'1'));
            assert_eq!(op.translate(128), Some(b'2'));
            assert_eq!(op.translate(255), Some(b'3'));
        }

        #[test]
        fn test_translate_operation_fallback() {
            // 测试当set1比set2长时的回退行为
            let mut op =
                TranslateOperation::new(vec![b'a', b'b', b'c', b'd'], vec![b'1', b'2'], false)
                    .unwrap();
            assert_eq!(op.translate(b'a'), Some(b'1'));
            assert_eq!(op.translate(b'b'), Some(b'2'));
            assert_eq!(op.translate(b'c'), Some(b'2')); // 使用set2的最后一个字符作为回退
            assert_eq!(op.translate(b'd'), Some(b'2')); // 使用set2的最后一个字符作为回退
        }

        #[test]
        fn test_complement_translation_complex() {
            // 测试复杂的补集转换
            let mut op = TranslateOperation::new(
                vec![b'a', b'e', b'i', b'o', b'u'],
                vec![b'A', b'E', b'I', b'O', b'U'],
                true,
            )
            .unwrap();

            // 在集合中的字符保持不变
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'e'), Some(b'e'));

            // 不在集合中的字符被映射
            assert_eq!(op.translate(b'b'), Some(b'A'));
            assert_eq!(op.translate(b'c'), Some(b'E'));
            assert_eq!(op.translate(b'd'), Some(b'I'));

            // 测试映射耗尽后的行为
            let mut chars = vec![];
            for i in 0..10 {
                chars.push(op.translate(b'x' + i).unwrap());
            }
            // 最后几个字符应该使用set2的最后一个字符
            assert!(chars.iter().any(|&c| c == b'U'));
        }

        #[test]
        fn test_next_complement_char() {
            // 测试基本功能
            let (next_iter, next_key) = TranslateOperation::next_complement_char(0, &[b'a', b'c']);
            assert_eq!(next_key, b'b');
            assert_eq!(next_iter, b'b' + 1);

            // 测试连续忽略
            let (next_iter, next_key) =
                TranslateOperation::next_complement_char(0, &[b'a', b'b', b'c']);
            assert_eq!(next_key, b'd');
            assert_eq!(next_iter, b'd' + 1);

            // 测试从中间开始
            let (next_iter, next_key) =
                TranslateOperation::next_complement_char(b'm', &[b'n', b'p']);
            assert_eq!(next_key, b'o');
            assert_eq!(next_iter, b'o' + 1);
        }
    }

    /// 测试压缩操作相关功能
    mod squeeze_operation_tests {
        use super::*;

        #[test]
        fn test_squeeze_operation() {
            // 测试基本压缩
            let mut op = SqueezeOperation::new(vec![b'a', b'b'], false);
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'a'), None); // 重复的 'a' 被删除
            assert_eq!(op.translate(b'b'), Some(b'b'));
            assert_eq!(op.translate(b'b'), None); // 重复的 'b' 被删除
            assert_eq!(op.translate(b'c'), Some(b'c'));
            assert_eq!(op.translate(b'c'), Some(b'c')); // 'c' 不在集合中，不被压缩

            // 测试补集压缩
            let mut op = SqueezeOperation::new(vec![b'a'], true);
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'a'), Some(b'a')); // 'a' 在集合中，不被压缩
            assert_eq!(op.translate(b'b'), Some(b'b'));
            assert_eq!(op.translate(b'b'), None); // 'b' 不在集合中，被压缩
        }

        #[test]
        fn test_squeeze_complex_cases() {
            // 测试混合字符序列
            let mut op = SqueezeOperation::new(vec![b'a', b'b'], false);
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'b'), Some(b'b'));
            assert_eq!(op.translate(b'c'), Some(b'c'));

            // 测试补集模式下的连续不同字符
            let mut op = SqueezeOperation::new(vec![b'x'], true);
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'b'), Some(b'b'));
            assert_eq!(op.translate(b'x'), Some(b'x'));
            assert_eq!(op.translate(b'x'), Some(b'x'));
            assert_eq!(op.translate(b'a'), Some(b'a'));
        }

        #[test]
        fn test_squeeze_basic_behavior() {
            // 测试基本的压缩行为
            let mut op = SqueezeOperation::new(vec![b'1'], false);

            // 第一个 '1' 应该保留
            assert_eq!(op.translate(b'1'), Some(b'1'));

            // 第二个 '1' 应该被压缩
            assert_eq!(op.translate(b'1'), None);

            // 其他字符应该保留
            assert_eq!(op.translate(b'a'), Some(b'a'));

            // 在其他字符后的 '1' 应该保留
            assert_eq!(op.translate(b'1'), Some(b'1'));

            // 再次出现的 '1' 应该被压缩
            assert_eq!(op.translate(b'1'), None);
        }

        #[test]
        fn test_squeeze_operation_with_special_chars() {
            // 测试压缩控制字符
            let mut op =
                SqueezeOperation::new(vec![unicodes::HT, unicodes::LF, unicodes::CR], false);
            assert_eq!(op.translate(unicodes::HT), Some(unicodes::HT));
            assert_eq!(op.translate(unicodes::HT), None); // 重复的制表符被压缩
            assert_eq!(op.translate(unicodes::LF), Some(unicodes::LF));
            assert_eq!(op.translate(unicodes::LF), None); // 重复的换行符被压缩
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'a'), Some(b'a')); // 普通字符不被压缩
        }

        #[test]
        fn test_squeeze_operation_with_large_set() {
            // 创建一个大的字符集
            let large_set: Vec<u8> = (0..128).collect();
            let mut op = SqueezeOperation::new(large_set, false);

            // 测试压缩范围内的字符
            for i in 0..128 {
                assert_eq!(op.translate(i), Some(i)); // 第一次出现保留
                assert_eq!(op.translate(i), None); // 第二次出现被压缩
            }

            // 测试范围外的字符
            for i in 128..=255 {
                assert_eq!(op.translate(i), Some(i)); // 第一次出现保留
                assert_eq!(op.translate(i), Some(i)); // 范围外的字符不被压缩
            }
        }

        #[test]
        fn test_squeeze_operation_complement_complex() {
            // 测试复杂的补集压缩
            let mut op = SqueezeOperation::new(vec![b'a', b'e', b'i', b'o', b'u'], true);

            // 在集合中的字符不被压缩
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'a'), Some(b'a'));
            assert_eq!(op.translate(b'e'), Some(b'e'));
            assert_eq!(op.translate(b'e'), Some(b'e'));

            // 不在集合中的字符被压缩
            assert_eq!(op.translate(b'b'), Some(b'b'));
            assert_eq!(op.translate(b'b'), None);
            assert_eq!(op.translate(b'c'), Some(b'c'));
            assert_eq!(op.translate(b'c'), None);

            // 混合序列
            assert_eq!(op.translate(b'a'), Some(b'a')); // 在集合中，不压缩
            assert_eq!(op.translate(b'b'), Some(b'b')); // 不在集合中，但与前一个不同，保留
            assert_eq!(op.translate(b'b'), None); // 不在集合中，与前一个相同，压缩
            assert_eq!(op.translate(b'a'), Some(b'a')); // 在集合中，不压缩
        }
    }

    /// 测试链式操作相关功能
    mod chain_operation_tests {
        use super::*;

        #[test]
        fn test_chain_operations() {
            // 测试删除后压缩
            let delete_op = DeleteOperation::new(vec![b'x'], false);
            let squeeze_op = SqueezeOperation::new(vec![b'a'], false);
            let mut chained = delete_op.chain(squeeze_op);

            assert_eq!(chained.translate(b'x'), None); // 'x' 被删除
            assert_eq!(chained.translate(b'a'), Some(b'a')); // 第一个 'a' 保留
            assert_eq!(chained.translate(b'a'), None); // 第二个 'a' 被压缩
            assert_eq!(chained.translate(b'b'), Some(b'b')); // 'b' 不变
        }

        #[test]
        fn test_complex_chain_operations() {
            // 测试三重链式操作：删除 -> 转换 -> 压缩
            let delete_op = DeleteOperation::new(vec![b'x'], false);
            let translate_op = TranslateOperation::new(vec![b'a'], vec![b'1'], false).unwrap();
            let squeeze_op = SqueezeOperation::new(vec![b'1'], false);

            let mut chained = delete_op.chain(translate_op).chain(squeeze_op);

            assert_eq!(chained.translate(b'x'), None); // 'x' 被删除
            assert_eq!(chained.translate(b'a'), Some(b'1')); // 'a' 被转换为 '1'

            // 根据 test_squeeze_basic_behavior 的结果，我们知道 SqueezeOperation 会压缩连续的相同字符
            // 所以当我们输入 b'1' 时，由于前一个字符已经是 '1'（从 'a' 转换而来），所以这个 '1' 会被压缩
            assert_eq!(chained.translate(b'1'), None); // 输入的 '1' 被压缩，因为前一个字符也是 '1'

            // 测试其他字符
            assert_eq!(chained.translate(b'b'), Some(b'b')); // 其他字符正常保留

            // 在其他字符后的 '1' 应该保留
            assert_eq!(chained.translate(b'1'), Some(b'1')); // 在 'b' 后的 '1' 保留

            // 再次出现的 '1' 应该被压缩
            assert_eq!(chained.translate(b'1'), None); // 连续的 '1' 被压缩
        }

        #[test]
        fn test_multi_level_chain() {
            // 测试多级链式操作
            let delete_op = DeleteOperation::new(vec![b' '], false); // 删除空格
            let translate_op = TranslateOperation::new(
                vec![b'a', b'e', b'i', b'o', b'u'],
                vec![b'A', b'E', b'I', b'O', b'U'],
                false,
            )
            .unwrap(); // 将元音转为大写
            let squeeze_op = SqueezeOperation::new(vec![b'A', b'E', b'I', b'O', b'U'], false); // 压缩大写元音

            // 三级链式操作
            let mut chained = delete_op.chain(translate_op).chain(squeeze_op);

            // 测试输入 "hello world"
            assert_eq!(chained.translate(b'h'), Some(b'h')); // 保持不变
            assert_eq!(chained.translate(b'e'), Some(b'E')); // 转为大写
            assert_eq!(chained.translate(b'l'), Some(b'l')); // 保持不变
            assert_eq!(chained.translate(b'l'), Some(b'l')); // 保持不变
            assert_eq!(chained.translate(b'o'), Some(b'O')); // 转为大写
            assert_eq!(chained.translate(b' '), None); // 空格被删除
            assert_eq!(chained.translate(b'w'), Some(b'w')); // 保持不变
            assert_eq!(chained.translate(b'o'), Some(b'O')); // 转为大写
            assert_eq!(chained.translate(b'r'), Some(b'r')); // 保持不变
            assert_eq!(chained.translate(b'l'), Some(b'l')); // 保持不变
            assert_eq!(chained.translate(b'd'), Some(b'd')); // 保持不变
        }

        #[test]
        fn test_chain_with_empty_operations() {
            // 测试空操作的链式
            let delete_op = DeleteOperation::new(vec![], false); // 不删除任何字符
            let translate_op = TranslateOperation::new(vec![], vec![], false).unwrap(); // 不转换任何字符
            let squeeze_op = SqueezeOperation::new(vec![], false); // 不压缩任何字符

            let mut chained = delete_op.chain(translate_op).chain(squeeze_op);

            // 所有字符应该保持不变
            for i in 0..128 {
                assert_eq!(chained.translate(i), Some(i));
            }
        }
    }
}
