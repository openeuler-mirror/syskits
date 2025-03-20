/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */
use std::error::Error;
use std::fmt::Display;
use std::io;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTError;

use crate::numberparse::ParseNumberError;

#[derive(Debug, PartialEq)]
pub enum SeqError {
    /// 解析输入参数时的错误
    ParseError(String, ParseNumberError),

    /// 增量参数为零的错误
    ZeroIncrement(String),

    /// 缺少必需的参数
    NoArguments,

    /// IO错误
    IoError(String),
}

impl CTError for SeqError {
    fn code(&self) -> i32 {
        match self {
            Self::NoArguments => 2,
            Self::ParseError(_, _) | Self::ZeroIncrement(_) => 1,
            Self::IoError(_) => 3,
        }
    }

    fn usage(&self) -> bool {
        matches!(self, Self::NoArguments)
    }
}

impl Error for SeqError {}

impl Display for SeqError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseError(s, e) => {
                let error_type = match e {
                    ParseNumberError::Float => "floating point",
                    ParseNumberError::Nan => "'not-a-number'",
                    ParseNumberError::Hex => "hexadecimal",
                };
                write!(f, "invalid {error_type} argument: {}", s.quote())
            }
            Self::ZeroIncrement(s) => write!(f, "invalid Zero increment value: {}", s.quote()),
            Self::NoArguments => write!(f, "missing operand"),
            Self::IoError(s) => write!(f, "IO error: {}", s),
        }
    }
}

impl From<io::Error> for SeqError {
    fn from(err: io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        assert_eq!(SeqError::NoArguments.code(), 2);
        assert_eq!(SeqError::ParseError("123".into(), ParseNumberError::Float).code(), 1);
        assert_eq!(SeqError::ZeroIncrement("0".into()).code(), 1);
        assert_eq!(SeqError::IoError("test".into()).code(), 3);
    }

    #[test]
    fn test_error_display() {
        assert_eq!(
            SeqError::ParseError("abc".into(), ParseNumberError::Float).to_string(),
            "invalid floating point argument: 'abc'"
        );
        assert_eq!(
            SeqError::ZeroIncrement("0".into()).to_string(),
            "invalid Zero increment value: '0'"
        );
        assert_eq!(SeqError::NoArguments.to_string(), "missing operand");
        assert_eq!(SeqError::IoError("test".into()).to_string(), "IO error: test");
    }

    #[test]
    fn test_error_usage() {
        assert!(SeqError::NoArguments.usage());
        assert!(!SeqError::ParseError("123".into(), ParseNumberError::Float).usage());
        assert!(!SeqError::ZeroIncrement("0".into()).usage());
        assert!(!SeqError::IoError("test".into()).usage());
    }
}
