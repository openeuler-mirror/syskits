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

#[derive(Debug)]
pub enum ParseError {
    ExpectedValue,
    Expected(String),
    ExtraArgument(String),
    MissingArgument(String),
    UnknownOperator(String),
    InvalidInteger(String),
    UnaryOperatorExpected(String),
}

/// A Result type for parsing test expressions
pub type ParseResult<T> = Result<T, ParseError>;

/// Implement Display trait for ParseError to make it easier to print useful errors.
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expected(s) => write!(f, "expected {s}"),
            Self::ExpectedValue => write!(f, "expected value"),
            Self::MissingArgument(s) => write!(f, "missing argument after {s}"),
            Self::ExtraArgument(s) => write!(f, "extra argument {s}"),
            Self::UnknownOperator(s) => write!(f, "unknown operator {s}"),
            Self::InvalidInteger(s) => write!(f, "invalid integer {s}"),
            Self::UnaryOperatorExpected(op) => write!(f, "{op}: unary operator expected"),
        }
    }
}

/// Implement UError trait for ParseError to make it easier to return useful error codes from main().
impl ctcore::ct_error::CTError for ParseError {
    fn code(&self) -> i32 {
        2
    }
}

/// Implement standard Error trait for UError
impl std::error::Error for ParseError {}
