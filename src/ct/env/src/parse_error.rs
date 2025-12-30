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

use std::fmt;

use crate::string_parser;

/// 定义一个错误类型，表示在尝试分割字符串参数时发生错误。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnvParseError {
    MissingClosingQuote {
        pos: usize,
        c: char,
    },
    InvalidBackslashAtEndOfStringInMinusS {
        pos: usize,
        quoting: String,
    },
    BackslashCNotAllowedInDoubleQuotes {
        pos: usize,
    },
    InvalidSequenceBackslashXInMinusS {
        pos: usize,
        c: char,
    },
    ParsingOfVariableNameFailed {
        pos: usize,
        msg: String,
    },
    InternalError {
        pos: usize,
        sub_err: string_parser::Error,
    },
    ReachedEnd,
    ContinueWithDelimiter,
}

impl fmt::Display for EnvParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(format!("{:?}", self).as_str())
    }
}

impl std::error::Error for EnvParseError {}

impl From<string_parser::Error> for EnvParseError {
    fn from(value: string_parser::Error) -> Self {
        Self::InternalError {
            pos: value.peek_position,
            sub_err: value,
        }
    }
}

#[test]
fn test_env_parse_error() {
    // Test MissingClosingQuote
    let error1 = EnvParseError::MissingClosingQuote { pos: 5, c: '"' };
    assert_eq!(
        "MissingClosingQuote { pos: 5, c: '\"' }",
        format!("{}", error1)
    );

    // Test InvalidBackslashAtEndOfStringInMinusS
    let error2 = EnvParseError::InvalidBackslashAtEndOfStringInMinusS {
        pos: 10,
        quoting: String::from("'"),
    };
    assert_eq!(
        "InvalidBackslashAtEndOfStringInMinusS { pos: 10, quoting: \"'\" }",
        format!("{}", error2)
    );

    // Test BackslashCNotAllowedInDoubleQuotes
    let error3 = EnvParseError::BackslashCNotAllowedInDoubleQuotes { pos: 15 };
    assert_eq!(
        "BackslashCNotAllowedInDoubleQuotes { pos: 15 }",
        format!("{}", error3)
    );

    // Test InvalidSequenceBackslashXInMinusS
    let error4 = EnvParseError::InvalidSequenceBackslashXInMinusS { pos: 20, c: 'a' };
    assert_eq!(
        "InvalidSequenceBackslashXInMinusS { pos: 20, c: 'a' }",
        format!("{}", error4)
    );

    // Test ParsingOfVariableNameFailed
    let error5 = EnvParseError::ParsingOfVariableNameFailed {
        pos: 25,
        msg: String::from("Invalid character"),
    };
    assert_eq!(
        "ParsingOfVariableNameFailed { pos: 25, msg: \"Invalid character\" }",
        format!("{}", error5)
    );

    // // Test InternalError
    // let error6 = EnvParseError::InternalError {
    //     pos: 30,
    //     sub_err: string_parser::Error::InvalidCharacter { pos: 30, c: '#' },
    // };
    // assert_eq!(
    //     "InternalError { pos: 30, sub_err: InvalidCharacter { pos: 30, c: '#' } }",
    //     format!("{}", error6)
    // );

    // Test ReachedEnd
    let error7 = EnvParseError::ReachedEnd;
    assert_eq!("ReachedEnd", format!("{}", error7));

    // Test ContinueWithDelimiter
    let error8 = EnvParseError::ContinueWithDelimiter;
    assert_eq!("ContinueWithDelimiter", format!("{}", error8));
}
