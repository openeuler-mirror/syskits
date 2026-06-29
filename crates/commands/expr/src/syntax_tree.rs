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

// expr 是一个经典的 Linux 或 Unix 命令行工具，用于执行基本的算术和逻辑表达式计算

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use onig::{EncodedBytes, Regex, RegexOptions, Region, SearchOptions, Syntax};

use crate::ExprError;
use crate::ExprResult;

// 定义支持的二元操作类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxTreeBinOp {
    Relation(SyntaxTreeRelationOp),
    Numeric(SyntaxTreeNumericOp),
    String(SyntaxTreeStringOp),
}

// 关系操作符枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxTreeRelationOp {
    Lt,  // 小于
    Leq, // 小于等于
    Eq,  // 等于
    Neq, // 不等于
    Gt,  // 大于
    Geq, // 大于等于
}

// 数字操作符枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxTreeNumericOp {
    Add, // 加法
    Sub, // 减法
    Mul, // 乘法
    Div, // 除法
    Mod, // 取模
}

// 字符串操作符枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxTreeStringOp {
    Match, // 匹配
    Index, // 索引
    And,   // 逻辑与
    Or,    // 逻辑或
}

// BinOp的实现，提供了计算方法
impl SyntaxTreeBinOp {
    /// 评估二元操作
    ///
    /// # 参数
    /// * `left` - 左操作数
    /// * `right` - 右操作数
    ///
    /// # 返回值
    /// 表示计算结果的 `ExprResult<NumOrStr>` 类型
    fn eval(
        &self,
        left: &SyntaxTreeAstNode,
        right: &SyntaxTreeAstNode,
    ) -> ExprResult<SyntaxTreeNumOrStr> {
        match self {
            Self::Relation(op) => op.eval(left, right),
            Self::Numeric(op) => op.eval(left, right),
            Self::String(op) => op.eval(left, right),
        }
    }
}

// 关系操作符的实现
impl SyntaxTreeRelationOp {
    /// 评估关系操作
    ///
    /// # 参数
    /// * `a` - 左操作数
    /// * `b` - 右操作数
    ///
    /// # 返回值
    /// 表示计算结果的 `ExprResult<NumOrStr>` 类型
    fn eval(
        &self,
        ast_a: &SyntaxTreeAstNode,
        ast_b: &SyntaxTreeAstNode,
    ) -> ExprResult<SyntaxTreeNumOrStr> {
        let a = ast_a.eval()?;
        let b = ast_b.eval()?;
        let is_b = if let (Some(a), Some(b)) = (a.to_bigint_strict(), b.to_bigint_strict()) {
            match self {
                Self::Lt => a < b,
                Self::Leq => a <= b,
                Self::Eq => a == b,
                Self::Neq => a != b,
                Self::Gt => a > b,
                Self::Geq => a >= b,
            }
        } else {
            // 使用字节序比较
            let a_bytes = a.to_bytes_for_compare();
            let b_bytes = b.to_bytes_for_compare();
            match self {
                Self::Lt => a_bytes < b_bytes,
                Self::Leq => a_bytes <= b_bytes,
                Self::Eq => a_bytes == b_bytes,
                Self::Neq => a_bytes != b_bytes,
                Self::Gt => a_bytes > b_bytes,
                Self::Geq => a_bytes >= b_bytes,
            }
        };
        if is_b { Ok(1.into()) } else { Ok(0.into()) }
    }
}

// 数字操作符的实现
impl SyntaxTreeNumericOp {
    /// 评估数字操作
    ///
    /// # 参数
    /// * `left` - 左操作数
    /// * `right` - 右操作数
    ///
    /// # 返回值
    /// 表示计算结果的 `ExprResult<NumOrStr>` 类型
    fn eval(
        &self,
        left: &SyntaxTreeAstNode,
        right: &SyntaxTreeAstNode,
    ) -> ExprResult<SyntaxTreeNumOrStr> {
        let a = left.eval()?.eval_as_bigint()?;
        let b = right.eval()?.eval_as_bigint()?;
        Ok(SyntaxTreeNumOrStr::Num(match self {
            Self::Add => a + b,
            Self::Sub => a - b,
            Self::Mul => a * b,
            Self::Div => match a.checked_div(&b) {
                Some(x) => x,
                None => return Err(ExprError::DivisionByZero),
            },
            Self::Mod => {
                if a.checked_div(&b).is_none() {
                    return Err(ExprError::DivisionByZero);
                };
                a % b
            }
        }))
    }
}

// 字符串操作符的实现
impl SyntaxTreeStringOp {
    /// 评估字符串操作
    ///
    /// # 参数
    /// * `left` - 左操作数
    /// * `right` - 右操作数
    ///
    /// # 返回值
    /// 表示计算结果的 `ExprResult<NumOrStr>` 类型
    fn eval(
        &self,
        left: &SyntaxTreeAstNode,
        right: &SyntaxTreeAstNode,
    ) -> ExprResult<SyntaxTreeNumOrStr> {
        match self {
            Self::Or => {
                let left = left.eval()?;
                if is_syntax_tree_truthy(&left) {
                    return Ok(left);
                }
                let right = right.eval()?;
                if is_syntax_tree_truthy(&right) {
                    return Ok(right);
                }
                Ok(0.into())
            }
            Self::And => {
                let left = left.eval()?;
                if !is_syntax_tree_truthy(&left) {
                    return Ok(0.into());
                }
                let right = right.eval()?;
                if !is_syntax_tree_truthy(&right) {
                    return Ok(0.into());
                }
                Ok(left)
            }
            Self::Match => {
                let left = left.eval()?.eval_as_bytes();
                let right = right.eval()?.eval_as_bytes();
                let normalized = normalize_bre_bytes(&right)?;
                let has_group = bre_has_capture_group(&normalized);
                let single_byte = is_single_byte_locale();

                if !single_byte && (!is_valid_utf8(&left) || !is_valid_utf8(&normalized)) {
                    return Ok(if has_group {
                        SyntaxTreeNumOrStr::Str(Vec::new())
                    } else {
                        SyntaxTreeNumOrStr::Num(BigInt::from(0))
                    });
                }

                let encoding = if single_byte {
                    // C/POSIX locale: single-byte with ASCII ctype semantics.
                    &raw mut onig_sys::OnigEncodingASCII
                } else {
                    &raw mut onig_sys::OnigEncodingUTF8
                };

                let re = Regex::with_options_and_encoding(
                    EncodedBytes::from_parts(&normalized, encoding),
                    RegexOptions::REGEX_OPTION_NONE,
                    Syntax::posix_basic(),
                )
                .map_err(|e| ExprError::RegexError(e.description().to_string()))?;

                let mut region = Region::new();
                let text = EncodedBytes::from_parts(&left, encoding);
                let matched = re.search_with_encoding(
                    text,
                    0,
                    left.len(),
                    SearchOptions::SEARCH_OPTION_NONE,
                    Some(&mut region),
                );

                let has_capture = re.captures_len() > 0;
                let match_pos = matched.and_then(|_| region.pos(0));
                let match_pos = match_pos.filter(|(start, _)| *start == 0);

                if let Some((start, end)) = match_pos {
                    if has_capture {
                        let capture = region
                            .pos(1)
                            .map(|(s, e)| left[s..e].to_vec())
                            .unwrap_or_default();
                        Ok(SyntaxTreeNumOrStr::Str(capture))
                    } else {
                        let match_len = if single_byte {
                            end - start
                        } else {
                            logical_len_bytes(&left[start..end])
                        };
                        Ok(SyntaxTreeNumOrStr::Num(BigInt::from(match_len)))
                    }
                } else if has_capture {
                    Ok(SyntaxTreeNumOrStr::Str(Vec::new()))
                } else {
                    Ok(SyntaxTreeNumOrStr::Num(BigInt::from(0)))
                }
            }
            Self::Index => {
                let left = left.eval()?.eval_as_bytes();
                let right = right.eval()?.eval_as_bytes();
                Ok(logical_index_bytes(&left, &right).into())
            }
        }
    }
}

/// 为二元操作符提供优先级定义
const PRECEDENCE: &[&[(&[u8], SyntaxTreeBinOp)]] = &[
    &[(b"|", SyntaxTreeBinOp::String(SyntaxTreeStringOp::Or))],
    &[(b"&", SyntaxTreeBinOp::String(SyntaxTreeStringOp::And))],
    &[
        (b"<", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Lt)),
        (b"<=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Leq)),
        (b"=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Eq)),
        (b"!=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Neq)),
        (b">=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Geq)),
        (b">", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Gt)),
    ],
    &[
        (b"+", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Add)),
        (b"-", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Sub)),
    ],
    &[
        (b"*", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Mul)),
        (b"/", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Div)),
        (b"%", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Mod)),
    ],
    &[(b":", SyntaxTreeBinOp::String(SyntaxTreeStringOp::Match))],
];

// 表示数值或字符串的类型
#[derive(Debug, Clone)] // 定义一个枚举，表示数值或者字符串类型
pub enum SyntaxTreeNumOrStr {
    Num(BigInt),
    Str(Vec<u8>),
}

// 从usize转换为NumOrStr
impl From<usize> for SyntaxTreeNumOrStr {
    fn from(num: usize) -> Self {
        Self::Num(BigInt::from(num))
    }
}

// 从BigInt转换为NumOrStr
impl From<BigInt> for SyntaxTreeNumOrStr {
    fn from(num: BigInt) -> Self {
        Self::Num(num)
    }
}

// 从Vec<u8>转换为NumOrStr
impl From<Vec<u8>> for SyntaxTreeNumOrStr {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Str(bytes)
    }
}

// 从String转换为NumOrStr
impl From<String> for SyntaxTreeNumOrStr {
    fn from(str: String) -> Self {
        Self::Str(str.into_bytes())
    }
}

// NumOrStr的实现部分
impl SyntaxTreeNumOrStr {
    // 尝试将NumOrStr转换为BigInt（严格整数格式）
    pub fn to_bigint_strict(&self) -> Option<BigInt> {
        match self {
            Self::Num(num) => Some(num.clone()),
            Self::Str(bytes) => parse_integer_strict_bytes(bytes),
        }
    }

    // 评估NumOrStr并尝试以BigInt形式返回
    pub fn eval_as_bigint(self) -> ExprResult<BigInt> {
        match self {
            Self::Num(num) => Ok(num),
            Self::Str(bytes) => {
                parse_integer_strict_bytes(&bytes).ok_or(ExprError::NonIntegerArgument)
            }
        }
    }

    // 评估NumOrStr并返回其字节表示
    pub fn eval_as_bytes(self) -> Vec<u8> {
        match self {
            Self::Num(num) => num.to_string().into_bytes(),
            Self::Str(bytes) => bytes,
        }
    }

    // 用于字符串比较的字节表示
    pub fn to_bytes_for_compare(&self) -> Vec<u8> {
        match self {
            Self::Num(num) => num.to_string().into_bytes(),
            Self::Str(bytes) => bytes.clone(),
        }
    }
}

// AstNode的实现部分，包含解析和评估逻辑
// 表示抽象语法树节点的枚举
#[derive(Debug, PartialEq, Eq)]
pub enum SyntaxTreeAstNode {
    // 叶子节点，包含一个字符串值
    Leaf {
        value: Vec<u8>,
    },
    // 二元操作节点，包含操作类型、左操作数和右操作数
    BinOp {
        op_type: SyntaxTreeBinOp,
        left: Box<SyntaxTreeAstNode>,
        right: Box<SyntaxTreeAstNode>,
    },
    // 字符串截取节点，包含目标字符串、起始位置和截取长度
    Substr {
        string: Box<SyntaxTreeAstNode>,
        pos: Box<SyntaxTreeAstNode>,
        length: Box<SyntaxTreeAstNode>,
    },
    // 字符串长度计算节点，包含目标字符串
    Length {
        string: Box<SyntaxTreeAstNode>,
    },
}

// AstNode的实现部分，包含解析和评估逻辑
impl SyntaxTreeAstNode {
    // 从字符切片解析为AstNode
    ///
    /// # 参数
    /// - `input`: 字符切片数组，代表待解析的表达式
    ///
    /// # 返回值
    /// - `ExprResult<SyntaxTreeAstNode>`: 解析成功返回表达式节点，失败则返回错误信息
    #[cfg(test)]
    pub fn parse(input: &[&str]) -> ExprResult<Self> {
        let bytes: Vec<Vec<u8>> = input.iter().map(|s| s.as_bytes().to_vec()).collect();
        Self::parse_bytes(&bytes)
    }

    pub fn parse_bytes(input: &[Vec<u8>]) -> ExprResult<Self> {
        SyntaxTreeParser::new(input).parse()
    }

    // 评估AstNode并返回NumOrStr类型的结果
    ///
    /// # 返回值
    /// - `ExprResult<SyntaxTreeNumOrStr>`: 评估成功返回数值或字符串结果，失败则返回错误信息
    pub fn eval(&self) -> ExprResult<SyntaxTreeNumOrStr> {
        match self {
            Self::Leaf { value } => Ok(value.clone().into()),
            Self::BinOp {
                op_type,
                left,
                right,
            } => op_type.eval(left, right),
            Self::Substr {
                string,
                pos,
                length,
            } => {
                // 实现字符串截取逻辑
                let string = string.eval()?.eval_as_bytes();
                let pos = pos
                    .eval()?
                    .eval_as_bigint()
                    .ok()
                    .and_then(|n| n.to_usize())
                    .unwrap_or(0);
                let length = length
                    .eval()?
                    .eval_as_bigint()
                    .ok()
                    .and_then(|n| n.to_usize())
                    .unwrap_or(0);

                // 检查并调整位置和长度以防止越界
                let Some(pos) = pos.checked_sub(1) else {
                    return Ok(String::new().into());
                };
                if length == 0 {
                    return Ok(String::new().into());
                }

                // 执行截取操作并返回结果
                Ok(logical_substr_bytes(&string, pos + 1, length).into())
            }
            Self::Length { string } => {
                Ok(logical_len_bytes(&string.eval()?.eval_as_bytes()).into())
            }
        }
    }
}

// 解析器结构体表示一个操作字符串切片的解析器。
// 它持有对输入的引用以及表示输入中当前位置的索引。
struct SyntaxTreeParser<'a> {
    input: &'a [Vec<u8>],
    index: usize,
}

// 新建创建一个带有给定输入的新 Parser 实例。
impl<'a> SyntaxTreeParser<'a> {
    fn new(input: &'a [Vec<u8>]) -> Self {
        Self { input, index: 0 }
    }

    // next 函数获取解析器的下一个 token。
    fn next(&mut self) -> ExprResult<&'a [u8]> {
        let next = self.input.get(self.index);
        if let Some(next) = next {
            self.index += 1;
            Ok(next.as_slice())
        } else {
            // 由于我们知道输入的大小大于零，因此索引不会引发恐慌。
            Err(ExprError::MissingArgument(bytes_to_string_lossy(
                &self.input[self.index - 1],
            )))
        }
    }

    // accept 函数尝试使用提供的函数 f 检查当前 token，并在成功时更新索引并返回结果。
    fn accept<T>(&mut self, f: impl Fn(&[u8]) -> Option<T>) -> Option<T> {
        let next = self.input.get(self.index)?;
        let tok = f(next.as_slice());
        if let Some(tok) = tok {
            self.index += 1;
            Some(tok)
        } else {
            None
        }
    }

    // parse 函数解析输入并构建表达式树。如果遇到未预期的参数，返回错误。
    fn parse(&mut self) -> ExprResult<SyntaxTreeAstNode> {
        // 如果输入为空，返回缺少操作数错误。
        if self.input.is_empty() {
            return Err(ExprError::MissingOperand);
        }
        let res = self.parse_expression()?;
        if let Some(arg) = self.input.get(self.index) {
            return Err(ExprError::UnexpectedArgument(bytes_to_string_lossy(arg)));
        }
        Ok(res)
    }

    // parse_expression 从最高优先级开始递归解析表达式。
    fn parse_expression(&mut self) -> ExprResult<SyntaxTreeAstNode> {
        self.parse_precedence(0)
    }

    // parse_op 根据给定的优先级检查当前 token 是否匹配运算符。
    fn parse_op(&mut self, precedence: usize) -> Option<SyntaxTreeBinOp> {
        self.accept(|s| {
            for (op_string, op) in PRECEDENCE[precedence] {
                if s == *op_string {
                    return Some(*op);
                }
            }
            None
        })
    }

    // parse_precedence 根据当前优先级解析表达式。
    fn parse_precedence(&mut self, precedence: usize) -> ExprResult<SyntaxTreeAstNode> {
        if precedence >= PRECEDENCE.len() {
            return self.parse_simple_expression();
        }

        let mut syntax_tree_ast_node_left = self.parse_precedence(precedence + 1)?;
        while let Some(syntax_tree_bin_op) = self.parse_op(precedence) {
            let right = self.parse_precedence(precedence + 1)?;
            syntax_tree_ast_node_left = SyntaxTreeAstNode::BinOp {
                op_type: syntax_tree_bin_op,
                left: Box::new(syntax_tree_ast_node_left),
                right: Box::new(right),
            };
        }
        Ok(syntax_tree_ast_node_left)
    }

    // parse_simple_expression 解析简单表达式，如匹配、子串、索引和长度操作。
    fn parse_simple_expression(&mut self) -> ExprResult<SyntaxTreeAstNode> {
        let first = self.next()?;
        Ok(match first {
            b"match" => {
                let syntax_tree_ast_node_left = self.parse_expression()?;
                let syntax_tree_ast_node_right = self.parse_expression()?;
                SyntaxTreeAstNode::BinOp {
                    op_type: SyntaxTreeBinOp::String(SyntaxTreeStringOp::Match),
                    left: Box::new(syntax_tree_ast_node_left),
                    right: Box::new(syntax_tree_ast_node_right),
                }
            }
            b"substr" => {
                let syntax_tree_ast_node = self.parse_expression()?;
                let pos = self.parse_expression()?;
                let length = self.parse_expression()?;
                SyntaxTreeAstNode::Substr {
                    string: Box::new(syntax_tree_ast_node),
                    pos: Box::new(pos),
                    length: Box::new(length),
                }
            }
            b"index" => {
                let syntax_tree_ast_node_left = self.parse_expression()?;
                let syntax_tree_ast_node_right = self.parse_expression()?;
                SyntaxTreeAstNode::BinOp {
                    op_type: SyntaxTreeBinOp::String(SyntaxTreeStringOp::Index),
                    left: Box::new(syntax_tree_ast_node_left),
                    right: Box::new(syntax_tree_ast_node_right),
                }
            }
            b"length" => {
                let syntax_tree_ast_node = self.parse_expression()?;
                SyntaxTreeAstNode::Length {
                    string: Box::new(syntax_tree_ast_node),
                }
            }
            b"+" => SyntaxTreeAstNode::Leaf {
                value: self.next()?.to_vec(),
            },
            b"(" => {
                let syntax_tree_ast_node = self.parse_expression()?;
                let close_paren = self.input.get(self.index).map(|v| v.as_slice());
                match close_paren {
                    None => {
                        return Err(ExprError::ExpectedClosingBraceAfter(bytes_to_string_lossy(
                            &self.input[self.index - 1],
                        )));
                    }
                    Some(b")") => {
                        self.index += 1;
                        syntax_tree_ast_node
                    }
                    Some(other) => {
                        return Err(ExprError::ExpectedClosingBraceInsteadOf(
                            bytes_to_string_lossy(other),
                        ));
                    }
                }
            }
            b")" => return Err(ExprError::UnexpectedClosingBrace),
            s => SyntaxTreeAstNode::Leaf { value: s.to_vec() },
        })
    }
}

/**
 * 判断NumOrStr类型的值是否为真。
 *
 * 对于Num类型，非零值被认为是真，零值被认为是假。
 * 对于Str类型，空字符串被认为是假，除此之外，以非零数字或非零字符开头的字符串被认为是真。
 * 特殊情况：字符串"-"被认为是真。
 *
 * @param s 要判断的NumOrStr类型的值。
 * @return 返回一个布尔值，表示输入值是否为真。
 */
pub fn is_syntax_tree_truthy(syntax_tree_str: &SyntaxTreeNumOrStr) -> bool {
    match syntax_tree_str {
        SyntaxTreeNumOrStr::Num(num) => num != &BigInt::from(0), // 对于数字，非零为真，零为假
        SyntaxTreeNumOrStr::Str(bytes) => {
            // 处理字符串类型的特殊情况："-"被视为真，空字符串被视为假
            if bytes == b"-" {
                return true;
            }

            let mut bytes = bytes.iter().copied();

            // 判断字符串是否为空，空则为假
            let Some(first) = bytes.next() else {
                return false;
            };

            // 判断字符串是否为只包含'0'的数字，是则为假；否则为真
            let is_zero = (first == b'-' || first == b'0') && bytes.all(|b| b == b'0');
            !is_zero
        }
    }
}

fn is_single_byte_locale() -> bool {
    let locale = std::env::var("LC_ALL")
        .ok()
        .or_else(|| std::env::var("LC_CTYPE").ok())
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase();
    locale == "C" || locale == "POSIX"
}

fn bytes_to_string_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn is_valid_utf8(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok()
}

fn utf8_char_len(bytes: &[u8], idx: usize) -> usize {
    let b0 = bytes[idx];
    if b0 < 0x80 {
        return 1;
    }
    if b0 < 0xC2 {
        return 1;
    }
    let len = bytes.len();
    let is_cont = |b: u8| (0x80..=0xBF).contains(&b);
    if b0 <= 0xDF {
        return if idx + 1 < len && is_cont(bytes[idx + 1]) {
            2
        } else {
            1
        };
    }
    if b0 == 0xE0 {
        return if idx + 2 < len
            && (0xA0..=0xBF).contains(&bytes[idx + 1])
            && is_cont(bytes[idx + 2])
        {
            3
        } else {
            1
        };
    }
    if (0xE1..=0xEC).contains(&b0) {
        return if idx + 2 < len && is_cont(bytes[idx + 1]) && is_cont(bytes[idx + 2]) {
            3
        } else {
            1
        };
    }
    if b0 == 0xED {
        return if idx + 2 < len
            && (0x80..=0x9F).contains(&bytes[idx + 1])
            && is_cont(bytes[idx + 2])
        {
            3
        } else {
            1
        };
    }
    if (0xEE..=0xEF).contains(&b0) {
        return if idx + 2 < len && is_cont(bytes[idx + 1]) && is_cont(bytes[idx + 2]) {
            3
        } else {
            1
        };
    }
    if b0 == 0xF0 {
        return if idx + 3 < len
            && (0x90..=0xBF).contains(&bytes[idx + 1])
            && is_cont(bytes[idx + 2])
            && is_cont(bytes[idx + 3])
        {
            4
        } else {
            1
        };
    }
    if (0xF1..=0xF3).contains(&b0) {
        return if idx + 3 < len
            && is_cont(bytes[idx + 1])
            && is_cont(bytes[idx + 2])
            && is_cont(bytes[idx + 3])
        {
            4
        } else {
            1
        };
    }
    if b0 == 0xF4 {
        return if idx + 3 < len
            && (0x80..=0x8F).contains(&bytes[idx + 1])
            && is_cont(bytes[idx + 2])
            && is_cont(bytes[idx + 3])
        {
            4
        } else {
            1
        };
    }
    1
}

fn logical_len_bytes(bytes: &[u8]) -> usize {
    if is_single_byte_locale() {
        bytes.len()
    } else {
        let mut i = 0;
        let mut count = 0;
        while i < bytes.len() {
            count += 1;
            i += utf8_char_len(bytes, i);
        }
        count
    }
}

fn logical_index_bytes(haystack: &[u8], needles: &[u8]) -> usize {
    if needles.is_empty() {
        return 0;
    }
    if is_single_byte_locale() {
        for (idx, b) in haystack.iter().enumerate() {
            if needles.contains(b) {
                return idx + 1;
            }
        }
        0
    } else {
        let mut needle_chars = Vec::new();
        let mut j = 0;
        while j < needles.len() {
            let len = utf8_char_len(needles, j);
            needle_chars.push(needles[j..j + len].to_vec());
            j += len;
        }

        let mut idx = 0;
        let mut i = 0;
        while i < haystack.len() {
            idx += 1;
            let len = utf8_char_len(haystack, i);
            let slice = &haystack[i..i + len];
            if needle_chars.iter().any(|n| n.as_slice() == slice) {
                return idx;
            }
            i += len;
        }
        0
    }
}

fn logical_substr_bytes(s: &[u8], pos: usize, len: usize) -> Vec<u8> {
    if pos == 0 || len == 0 {
        return Vec::new();
    }
    if is_single_byte_locale() {
        let start = pos - 1;
        if start >= s.len() {
            return Vec::new();
        }
        let end = std::cmp::min(start + len, s.len());
        s[start..end].to_vec()
    } else {
        let mut out = Vec::new();
        let mut idx = 1;
        let mut i = 0;
        let mut remaining = len;
        while i < s.len() && remaining > 0 {
            let char_len = utf8_char_len(s, i);
            if idx >= pos {
                out.extend_from_slice(&s[i..i + char_len]);
                remaining -= 1;
            }
            idx += 1;
            i += char_len;
        }
        out
    }
}

fn parse_integer_strict_bytes(bytes: &[u8]) -> Option<BigInt> {
    if bytes.is_empty() {
        return None;
    }
    let (start, negative) = if bytes[0] == b'-' {
        (1, true)
    } else {
        (0, false)
    };
    if start >= bytes.len() {
        return None;
    }
    if !bytes[start..].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let num = BigInt::parse_bytes(&bytes[start..], 10)?;
    Some(if negative { -num } else { num })
}

fn bre_has_capture_group(pattern: &[u8]) -> bool {
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            b'\\' => {
                if i + 1 < pattern.len() && pattern[i + 1] == b'(' {
                    return true;
                }
                i += 2;
            }
            b'[' => {
                let (_, next_index) = read_char_class_bytes(pattern, i);
                i = next_index;
            }
            _ => {
                i += 1;
            }
        }
    }
    false
}

fn normalize_bre_bytes(pattern: &[u8]) -> ExprResult<Vec<u8>> {
    let mut i = 0;
    let mut depth = 0usize;
    let mut out = Vec::with_capacity(pattern.len());
    let mut last_can_repeat = false;

    while i < pattern.len() {
        if pattern[i] == b'\\' {
            if i + 1 >= pattern.len() {
                out.push(b'\\');
                break;
            }
            let next = pattern[i + 1];
            match next {
                b'(' => {
                    depth += 1;
                    out.extend_from_slice(b"\\(");
                    last_can_repeat = false;
                    i += 2;
                    continue;
                }
                b')' => {
                    if depth == 0 {
                        return Err(ExprError::RegexError("Unmatched ) or \\)".to_string()));
                    }
                    depth -= 1;
                    out.extend_from_slice(b"\\)");
                    last_can_repeat = true;
                    i += 2;
                    continue;
                }
                b'{' => {
                    let (content, next_index) = read_bre_brace_bytes(pattern, i + 2)?;
                    if !last_can_repeat {
                        out.push(b'{');
                        out.extend_from_slice(&content);
                        out.push(b'}');
                        last_can_repeat = true;
                    } else {
                        let normalized = normalize_brace_content_bytes(&content)?;
                        out.extend_from_slice(b"\\{");
                        out.extend_from_slice(&normalized);
                        out.extend_from_slice(b"\\}");
                        last_can_repeat = true;
                    }
                    i = next_index;
                    continue;
                }
                b'|' => {
                    out.extend_from_slice(b"\\|");
                    last_can_repeat = false;
                    i += 2;
                    continue;
                }
                _ => {
                    out.push(b'\\');
                    out.push(next);
                    last_can_repeat = true;
                    i += 2;
                    continue;
                }
            }
        }

        match pattern[i] {
            b'^' | b'$' => {
                out.push(pattern[i]);
                last_can_repeat = false;
            }
            b'*' => {
                if last_can_repeat {
                    out.push(b'*');
                } else {
                    out.extend_from_slice(b"\\*");
                    last_can_repeat = true;
                }
            }
            b'[' => {
                let (class, next_index) = read_char_class_bytes(pattern, i);
                out.extend_from_slice(&class);
                last_can_repeat = true;
                i = next_index;
                continue;
            }
            _ => {
                out.push(pattern[i]);
                last_can_repeat = true;
            }
        }
        i += 1;
    }

    if depth > 0 {
        return Err(ExprError::RegexError("Unmatched ( or \\(".to_string()));
    }
    Ok(out)
}

fn read_bre_brace_bytes(pattern: &[u8], start: usize) -> ExprResult<(Vec<u8>, usize)> {
    let mut i = start;
    while i + 1 < pattern.len() {
        if pattern[i] == b'\\' && pattern[i + 1] == b'}' {
            let content = pattern[start..i].to_vec();
            if !is_valid_brace_content_bytes(&content) {
                return Err(ExprError::RegexError(
                    "Invalid content of \\{\\}".to_string(),
                ));
            }
            return Ok((content, i + 2));
        }
        i += 1;
    }
    Err(ExprError::RegexError("Unmatched \\{".to_string()))
}

fn read_char_class_bytes(pattern: &[u8], start: usize) -> (Vec<u8>, usize) {
    let mut i = start + 1;
    let mut escaped = false;
    while i < pattern.len() {
        let ch = pattern[i];
        if escaped {
            escaped = false;
        } else if ch == b'\\' {
            escaped = true;
        } else if ch == b']' {
            return (pattern[start..i + 1].to_vec(), i + 1);
        }
        i += 1;
    }
    (pattern[start..].to_vec(), pattern.len())
}

fn is_valid_brace_content_bytes(content: &[u8]) -> bool {
    let mut parts = content.split(|b| *b == b',');
    let first = parts.next();
    let second = parts.next();
    if parts.next().is_some() {
        return false;
    }
    match (first, second) {
        (Some(a), Some(b)) => {
            if !a.is_empty() && !a.iter().all(|c| c.is_ascii_digit()) {
                return false;
            }
            if !b.is_empty() && !b.iter().all(|c| c.is_ascii_digit()) {
                return false;
            }
            let low = parse_bound_bytes(a);
            let high = parse_bound_bytes(b);
            if low.is_none() || high.is_none() {
                return false;
            }
            if !a.is_empty() && !b.is_empty() && high.unwrap() < low.unwrap() {
                return false;
            }
            true
        }
        (Some(a), None) => {
            if a.is_empty() {
                return false;
            }
            if !a.iter().all(|c| c.is_ascii_digit()) {
                return false;
            }
            parse_bound_bytes(a).is_some()
        }
        (None, None) => false,
        (None, Some(_)) => false,
    }
}

fn normalize_brace_content_bytes(content: &[u8]) -> ExprResult<Vec<u8>> {
    let mut parts = content.split(|b| *b == b',');
    let first = parts.next().unwrap_or(&[]);
    let second = parts.next();
    if parts.next().is_some() {
        return Err(ExprError::RegexError(
            "Invalid content of \\{\\}".to_string(),
        ));
    }
    match second {
        None => Ok(first.to_vec()),
        Some(b) => {
            if first.is_empty() {
                let mut out = Vec::with_capacity(2 + b.len());
                out.extend_from_slice(b"0,");
                out.extend_from_slice(b);
                Ok(out)
            } else {
                let mut out = Vec::with_capacity(first.len() + 1 + b.len());
                out.extend_from_slice(first);
                out.push(b',');
                out.extend_from_slice(b);
                Ok(out)
            }
        }
    }
}

fn parse_bound_bytes(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return Some(0);
    }
    let mut val: u32 = 0;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.saturating_mul(10).saturating_add((b - b'0') as u32);
        if val > 32767 {
            return None;
        }
    }
    Some(val)
}

#[cfg(test)]
mod test {
    use super::{
        SyntaxTreeAstNode, SyntaxTreeBinOp, SyntaxTreeNumericOp, SyntaxTreeRelationOp,
        SyntaxTreeStringOp,
    };

    impl From<&str> for SyntaxTreeAstNode {
        fn from(value: &str) -> Self {
            Self::Leaf {
                value: value.as_bytes().to_vec(),
            }
        }
    }

    fn op(
        op_type: SyntaxTreeBinOp,
        left: impl Into<SyntaxTreeAstNode>,
        right: impl Into<SyntaxTreeAstNode>,
    ) -> SyntaxTreeAstNode {
        SyntaxTreeAstNode::BinOp {
            op_type,
            left: Box::new(left.into()),
            right: Box::new(right.into()),
        }
    }

    fn length(string: impl Into<SyntaxTreeAstNode>) -> SyntaxTreeAstNode {
        SyntaxTreeAstNode::Length {
            string: Box::new(string.into()),
        }
    }

    fn substr(
        string: impl Into<SyntaxTreeAstNode>,
        pos: impl Into<SyntaxTreeAstNode>,
        length: impl Into<SyntaxTreeAstNode>,
    ) -> SyntaxTreeAstNode {
        SyntaxTreeAstNode::Substr {
            string: Box::new(string.into()),
            pos: Box::new(pos.into()),
            length: Box::new(length.into()),
        }
    }

    #[test]
    fn infix_operators() {
        let cases = [
            ("|", SyntaxTreeBinOp::String(SyntaxTreeStringOp::Or)),
            ("&", SyntaxTreeBinOp::String(SyntaxTreeStringOp::And)),
            ("<", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Lt)),
            ("<=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Leq)),
            ("=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Eq)),
            ("!=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Neq)),
            (">=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Geq)),
            (">", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Gt)),
            ("+", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Add)),
            ("-", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Sub)),
            ("*", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Mul)),
            ("/", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Div)),
            ("%", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Mod)),
            (":", SyntaxTreeBinOp::String(SyntaxTreeStringOp::Match)),
        ];
        for (string, value) in cases {
            assert_eq!(
                SyntaxTreeAstNode::parse(&["1", string, "2"]),
                Ok(op(value, "1", "2"))
            );
        }
    }

    #[test]
    fn other_operators() {
        assert_eq!(
            SyntaxTreeAstNode::parse(&["match", "1", "2"]),
            Ok(op(
                SyntaxTreeBinOp::String(SyntaxTreeStringOp::Match),
                "1",
                "2"
            )),
        );
        assert_eq!(
            SyntaxTreeAstNode::parse(&["index", "1", "2"]),
            Ok(op(
                SyntaxTreeBinOp::String(SyntaxTreeStringOp::Index),
                "1",
                "2"
            )),
        );
        assert_eq!(SyntaxTreeAstNode::parse(&["length", "1"]), Ok(length("1")),);
        assert_eq!(
            SyntaxTreeAstNode::parse(&["substr", "1", "2", "3"]),
            Ok(substr("1", "2", "3")),
        );
    }

    #[test]
    fn precedence() {
        assert_eq!(
            SyntaxTreeAstNode::parse(&["1", "+", "2", "*", "3"]),
            Ok(op(
                SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Add),
                "1",
                op(SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Mul), "2", "3")
            ))
        );
        assert_eq!(
            SyntaxTreeAstNode::parse(&["(", "1", "+", "2", ")", "*", "3"]),
            Ok(op(
                SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Mul),
                op(SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Add), "1", "2"),
                "3"
            ))
        );
        assert_eq!(
            SyntaxTreeAstNode::parse(&["1", "*", "2", "+", "3"]),
            Ok(op(
                SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Add),
                op(SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Mul), "1", "2"),
                "3"
            )),
        );
    }
}
