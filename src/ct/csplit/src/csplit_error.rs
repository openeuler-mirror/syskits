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
use std::io;
use thiserror::Error;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTError;

/// Errors thrown by the csplit command
#[derive(Debug, Error)]
pub enum CsplitError {
    #[error("IO error: {}", _0)]
    IoError(io::Error),
    #[error("{}: line number out of range", ._0.quote())]
    LineOutOfRange(String),
    #[error("{}: line number out of range on repetition {}", ._0.quote(), _1)]
    LineOutOfRangeOnRepetition(String, usize),
    #[error("{}: match not found", ._0.quote())]
    MatchNotFound(String),
    #[error("{}: match not found on repetition {}", ._0.quote(), _1)]
    MatchNotFoundOnRepetition(String, usize),
    #[error("0: line number must be greater than zero")]
    LineNumberIsZero,
    #[error("line number '{}' is smaller than preceding line number, {}", _0, _1)]
    LineNumberSmallerThanPrevious(usize, usize),
    #[error("{}: invalid pattern", ._0.quote())]
    InvalidPattern(String),
    #[error("invalid number: {}", ._0.quote())]
    InvalidNumber(String),
    #[error("incorrect conversion specification in suffix")]
    SuffixFormatIncorrect,
    #[error("too many % conversion specifications in suffix")]
    SuffixFormatTooManyPercents,
    #[error("{} is not a regular file", ._0.quote())]
    NotRegularFile(String),
}

impl From<io::Error> for CsplitError {
    fn from(error: io::Error) -> Self {
        Self::IoError(error)
    }
}

impl CTError for CsplitError {
    fn code(&self) -> i32 {
        1
    }
}

mod tests_csplit {
    #[test]
    fn test_csplit_error_io_error() {
        use crate::csplit_error::CsplitError;
        let error = std::io::Error::new(std::io::ErrorKind::Other, "IO error");
        let csplit_error = CsplitError::from(error);

        assert_eq!(csplit_error.to_string(), "IO error: IO error");
    }

    #[test]
    fn test_csplit_error_line_out_of_range() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::LineOutOfRange("file.txt".into());

        assert_eq!(error.to_string(), "'file.txt': line number out of range");
    }

    #[test]
    fn test_csplit_error_line_out_of_range_on_repetition() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::LineOutOfRangeOnRepetition("file.txt".into(), 3);

        assert_eq!(
            error.to_string(),
            "'file.txt': line number out of range on repetition 3"
        );
    }

    #[test]
    fn test_csplit_error_match_not_found() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::MatchNotFound("pattern".into());

        assert_eq!(error.to_string(), "'pattern': match not found");
    }

    #[test]
    fn test_csplit_error_match_not_found_on_repetition() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::MatchNotFoundOnRepetition("pattern".into(), 2);

        assert_eq!(
            error.to_string(),
            "'pattern': match not found on repetition 2"
        );
    }

    #[test]
    fn test_csplit_error_line_number_is_zero() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::LineNumberIsZero;

        assert_eq!(
            error.to_string(),
            "0: line number must be greater than zero"
        );
    }

    #[test]
    fn test_csplit_error_line_number_smaller_than_previous() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::LineNumberSmallerThanPrevious(5, 3);

        assert_eq!(
            error.to_string(),
            "line number '5' is smaller than preceding line number, 3"
        );
    }

    #[test]
    fn test_csplit_error_invalid_pattern() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::InvalidPattern("invalid".into());

        assert_eq!(error.to_string(), "'invalid': invalid pattern");
    }

    #[test]
    fn test_csplit_error_invalid_number() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::InvalidNumber("123".into());

        assert_eq!(error.to_string(), "invalid number: \'123\'");
    }

    #[test]
    fn test_csplit_error_suffix_format_incorrect() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::SuffixFormatIncorrect;

        assert_eq!(
            error.to_string(),
            "incorrect conversion specification in suffix"
        );
    }

    #[test]
    fn test_csplit_error_suffix_format_too_many_percents() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::SuffixFormatTooManyPercents;

        assert_eq!(
            error.to_string(),
            "too many % conversion specifications in suffix"
        );
    }

    #[test]
    fn test_csplit_error_not_regular_file() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::NotRegularFile("file.txt".into());

        assert_eq!(error.to_string(), "'file.txt' is not a regular file");
    }
}
