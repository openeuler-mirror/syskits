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

// expr 是一个经典的 Linux 或 Unix 命令行工具，用于执行基本的算术和逻辑表达式计算

use num_bigint::BigInt;
use num_bigint::ParseBigIntError;
use num_traits::ToPrimitive;
use onig::Regex;
use onig::RegexOptions;
use onig::Syntax;

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
        let is_b = if let (Ok(a), Ok(b)) = (&a.to_bigint(), &b.to_bigint()) {
            match self {
                Self::Lt => a < b,
                Self::Leq => a <= b,
                Self::Eq => a == b,
                Self::Neq => a != b,
                Self::Gt => a > b,
                Self::Geq => a >= b,
            }
        } else {
            // 使用字符串比较
            let a = a.eval_as_string();
            let b = b.eval_as_string();
            match self {
                Self::Lt => a < b,
                Self::Leq => a <= b,
                Self::Eq => a == b,
                Self::Neq => a != b,
                Self::Gt => a > b,
                Self::Geq => a >= b,
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
                let left = left.eval()?.eval_as_string();
                let right = right.eval()?.eval_as_string();
                let re_string = format!("^{}", right);
                let re = Regex::with_options(
                    &re_string,
                    RegexOptions::REGEX_OPTION_NONE,
                    Syntax::grep(),
                )
                .map_err(|_| ExprError::InvalidRegexExpression)?;
                Ok(if re.captures_len() > 0 {
                    re.captures(&left)
                        .map(|captures| captures.at(1).unwrap())
                        .unwrap_or("")
                        .to_string()
                } else {
                    re.find(&left)
                        .map_or("0".to_string(), |(start, end)| (end - start).to_string())
                }
                .into())
            }
            Self::Index => {
                let left = left.eval()?.eval_as_string();
                let right = right.eval()?.eval_as_string();
                for (current_idx, ch_h) in left.chars().enumerate() {
                    for ch_n in right.to_string().chars() {
                        if ch_n == ch_h {
                            return Ok((current_idx + 1).into());
                        }
                    }
                }
                Ok(0.into())
            }
        }
    }
}

/// 为二元操作符提供优先级定义
const PRECEDENCE: &[&[(&str, SyntaxTreeBinOp)]] = &[
    &[("|", SyntaxTreeBinOp::String(SyntaxTreeStringOp::Or))],
    &[("&", SyntaxTreeBinOp::String(SyntaxTreeStringOp::And))],
    &[
        ("<", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Lt)),
        ("<=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Leq)),
        ("=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Eq)),
        ("!=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Neq)),
        (">=", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Geq)),
        (">", SyntaxTreeBinOp::Relation(SyntaxTreeRelationOp::Gt)),
    ],
    &[
        ("+", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Add)),
        ("-", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Sub)),
    ],
    &[
        ("*", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Mul)),
        ("/", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Div)),
        ("%", SyntaxTreeBinOp::Numeric(SyntaxTreeNumericOp::Mod)),
    ],
    &[(":", SyntaxTreeBinOp::String(SyntaxTreeStringOp::Match))],
];

// 表示数值或字符串的类型
#[derive(Debug)] // 定义一个枚举，表示数值或者字符串类型
pub enum SyntaxTreeNumOrStr {
    Num(BigInt),
    Str(String),
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

// 从String转换为NumOrStr
impl From<String> for SyntaxTreeNumOrStr {
    fn from(str: String) -> Self {
        Self::Str(str)
    }
}

// NumOrStr的实现部分
impl SyntaxTreeNumOrStr {
    // 尝试将NumOrStr转换为BigInt
    pub fn to_bigint(&self) -> Result<BigInt, ParseBigIntError> {
        match self {
            Self::Num(num) => Ok(num.clone()),
            Self::Str(str) => str.parse::<BigInt>(),
        }
    }

    // 评估NumOrStr并尝试以BigInt形式返回
    pub fn eval_as_bigint(self) -> ExprResult<BigInt> {
        match self {
            Self::Num(num) => Ok(num),
            Self::Str(str) => str
                .parse::<BigInt>()
                .map_err(|_| ExprError::NonIntegerArgument),
        }
    }

    // 评估NumOrStr并返回其字符串表示
    pub fn eval_as_string(self) -> String {
        match self {
            Self::Num(num) => num.to_string(),
            Self::Str(str) => str,
        }
    }
}

// AstNode的实现部分，包含解析和评估逻辑
// 表示抽象语法树节点的枚举
#[derive(Debug, PartialEq, Eq)]
pub enum SyntaxTreeAstNode {
    // 叶子节点，包含一个字符串值
    Leaf {
        value: String,
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
    pub fn parse(input: &[&str]) -> ExprResult<Self> {
        SyntaxTreeParser::new(input).parse()
    }

    // 评估AstNode并返回NumOrStr类型的结果
    ///
    /// # 返回值
    /// - `ExprResult<SyntaxTreeNumOrStr>`: 评估成功返回数值或字符串结果，失败则返回错误信息
    pub fn eval(&self) -> ExprResult<SyntaxTreeNumOrStr> {
        match self {
            Self::Leaf { value } => Ok(value.to_string().into()),
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
                let string: String = string.eval()?.eval_as_string();
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
                let (Some(pos), Some(_)) = (pos.checked_sub(1), length.checked_sub(1)) else {
                    return Ok(String::new().into());
                };

                // 执行截取操作并返回结果
                Ok(string
                    .chars()
                    .skip(pos)
                    .take(length)
                    .collect::<String>()
                    .into())
            }
            Self::Length { string } => Ok(string.eval()?.eval_as_string().chars().count().into()),
        }
    }
}

// 解析器结构体表示一个操作字符串切片的解析器。
// 它持有对输入的引用以及表示输入中当前位置的索引。
struct SyntaxTreeParser<'a> {
    input: &'a [&'a str],
    index: usize,
}

// 新建创建一个带有给定输入的新 Parser 实例。
impl<'a> SyntaxTreeParser<'a> {
    fn new(input: &'a [&'a str]) -> Self {
        Self { input, index: 0 }
    }

    // next 函数获取解析器的下一个 token。
    fn next(&mut self) -> ExprResult<&'a str> {
        let next = self.input.get(self.index);
        if let Some(next) = next {
            self.index += 1;
            Ok(next)
        } else {
            // 由于我们知道输入的大小大于零，因此索引不会引发恐慌。
            Err(ExprError::MissingArgument(
                self.input[self.index - 1].into(),
            ))
        }
    }

    // accept 函数尝试使用提供的函数 f 检查当前 token，并在成功时更新索引并返回结果。
    fn accept<T>(&mut self, f: impl Fn(&str) -> Option<T>) -> Option<T> {
        let next = self.input.get(self.index)?;
        let tok = f(next);
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
            return Err(ExprError::UnexpectedArgument(arg.to_string()));
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
            "match" => {
                let syntax_tree_ast_node_left = self.parse_expression()?;
                let syntax_tree_ast_node_right = self.parse_expression()?;
                SyntaxTreeAstNode::BinOp {
                    op_type: SyntaxTreeBinOp::String(SyntaxTreeStringOp::Match),
                    left: Box::new(syntax_tree_ast_node_left),
                    right: Box::new(syntax_tree_ast_node_right),
                }
            }
            "substr" => {
                let syntax_tree_ast_node = self.parse_expression()?;
                let pos = self.parse_expression()?;
                let length = self.parse_expression()?;
                SyntaxTreeAstNode::Substr {
                    string: Box::new(syntax_tree_ast_node),
                    pos: Box::new(pos),
                    length: Box::new(length),
                }
            }
            "index" => {
                let syntax_tree_ast_node_left = self.parse_expression()?;
                let syntax_tree_ast_node_right = self.parse_expression()?;
                SyntaxTreeAstNode::BinOp {
                    op_type: SyntaxTreeBinOp::String(SyntaxTreeStringOp::Index),
                    left: Box::new(syntax_tree_ast_node_left),
                    right: Box::new(syntax_tree_ast_node_right),
                }
            }
            "length" => {
                let syntax_tree_ast_node = self.parse_expression()?;
                SyntaxTreeAstNode::Length {
                    string: Box::new(syntax_tree_ast_node),
                }
            }
            "+" => SyntaxTreeAstNode::Leaf {
                value: self.next()?.into(),
            },
            "(" => {
                let syntax_tree_ast_node = self.parse_expression()?;
                let close_paren = self.next()?;
                if close_paren != ")" {
                    // 由于我们已经解析了至少一个'('，所以在 `self.index - 1` 处会有 token。
                    // 因此，此索引不会引发恐慌。
                    return Err(ExprError::ExpectedClosingBraceAfter(
                        self.input[self.index - 1].into(),
                    ));
                }
                syntax_tree_ast_node
            }
            s => SyntaxTreeAstNode::Leaf { value: s.into() },
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
        SyntaxTreeNumOrStr::Str(str) => {
            // 处理字符串类型的特殊情况："-"被视为真，空字符串被视为假
            if str == "-" {
                return true;
            }

            let mut bytes = str.bytes();

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

#[cfg(test)]
mod test {
    use super::{
        SyntaxTreeAstNode, SyntaxTreeBinOp, SyntaxTreeNumericOp, SyntaxTreeRelationOp,
        SyntaxTreeStringOp,
    };

    impl From<&str> for SyntaxTreeAstNode {
        fn from(value: &str) -> Self {
            Self::Leaf {
                value: value.into(),
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