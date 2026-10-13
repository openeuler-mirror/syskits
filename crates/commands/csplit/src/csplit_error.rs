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
use std::ffi::OsString;
use std::io;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
use thiserror::Error;

use ctcore::ct_display::{Quotable, locale_quote};
use ctcore::ct_error::CTError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatternText(Vec<u8>);

impl PatternText {
    pub fn locale_quote(&self) -> String {
        if let Ok(text) = std::str::from_utf8(&self.0) {
            return locale_quote(text);
        }

        #[cfg(unix)]
        return super::locale_quote_file_operand(&OsString::from_vec(self.0.clone()));

        #[cfg(not(unix))]
        locale_quote(&String::from_utf8_lossy(&self.0))
    }

    pub fn to_lossy_string(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

impl From<&str> for PatternText {
    fn from(value: &str) -> Self {
        Self(value.as_bytes().to_owned())
    }
}

impl From<String> for PatternText {
    fn from(value: String) -> Self {
        Self(value.into_bytes())
    }
}

impl From<Vec<u8>> for PatternText {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl PartialEq<&str> for PatternText {
    fn eq(&self, other: &&str) -> bool {
        self.0 == other.as_bytes()
    }
}

/// Errors thrown by the csplit command
#[derive(Debug, Error)]
pub enum CsplitError {
    #[error("IO error: {}", _0)]
    IoError(io::Error),
    #[error("{source}")]
    DeferredOutput {
        source: Box<CsplitError>,
        stdout_lines: Vec<String>,
    },
    #[error("{}: {}", _0, _1)]
    WriteError(String, String),
    #[error("write error: {}", _0)]
    StdoutWriteError(String),
    #[error("{}: {}", ._0.maybe_quote(), _1)]
    OutputCreateError(String, String),
    #[error("input disappeared")]
    InputDisappeared,
    #[error("{}: line number out of range", ._0.locale_quote())]
    LineOutOfRange(PatternText),
    #[error(
        "{}: line number out of range on repetition {}",
        ._0.locale_quote(),
        _1
    )]
    LineOutOfRangeOnRepetition(PatternText, u64),
    #[error("{}: match not found", ._0.locale_quote())]
    MatchNotFound(PatternText),
    #[error("{}: match not found on repetition {}", ._0.locale_quote(), _1)]
    MatchNotFoundOnRepetition(PatternText, u64),
    #[error("0: line number must be greater than zero")]
    LineNumberIsZero,
    #[error("line number '{}' is smaller than preceding line number, {}", _0, _1)]
    LineNumberSmallerThanPrevious(usize, usize),
    #[error("{}: invalid pattern", ._0.locale_quote())]
    InvalidPattern(PatternText),
    #[error("{}: invalid regular expression: {}", ._0.locale_quote(), _1)]
    InvalidRegularExpression(PatternText, String),
    #[error("{}: closing delimiter '{}' missing", _0, _1)]
    ClosingDelimiterMissing(String, char),
    #[error("{}: integer expected after delimiter", ._0.locale_quote())]
    IntegerExpectedAfterDelimiter(PatternText),
    #[error("{}: '}}' is required in repeat count", locale_quote(&._0))]
    RepeatCountMissingClosingBrace(String),
    #[error("{}: integer required between '{{' and '}}'", _0)]
    RepeatCountIntegerRequired(String),
    #[error("invalid number: {}", locale_quote(&._0))]
    InvalidNumber(String),
    #[error(
        "invalid number: {}: Numerical result out of range",
        locale_quote(&._0)
    )]
    InvalidNumberOutOfRange(String),
    #[error(
        "invalid number: {}: Value too large for defined data type",
        locale_quote(&._0)
    )]
    InvalidNumberTooLarge(String),
    #[error("missing % conversion specification in suffix")]
    SuffixFormatMissingConversion,
    #[error("missing conversion specifier in suffix")]
    SuffixFormatMissingSpecifier,
    #[error("invalid conversion specifier in suffix: {}", _0)]
    SuffixFormatInvalidSpecifier(String),
    #[error("invalid flags in conversion specification: {}", _0)]
    SuffixFormatInvalidFlags(String),
    #[error("incorrect conversion specification in suffix")]
    SuffixFormatIncorrect,
    #[error("too many % conversion specifications in suffix")]
    SuffixFormatTooManyPercents,
    // #[error("{} is not a regular file", ._0.quote())]
    // NotRegularFile(String),
}

impl CsplitError {
    pub fn with_deferred_stdout(source: Self, stdout_lines: Vec<String>) -> Self {
        if stdout_lines.is_empty() {
            source
        } else {
            Self::DeferredOutput {
                source: Box::new(source),
                stdout_lines,
            }
        }
    }

    pub fn is_match_not_found(&self) -> bool {
        match self {
            Self::MatchNotFound(_) => true,
            Self::DeferredOutput { source, .. } => source.is_match_not_found(),
            _ => false,
        }
    }

    pub fn preserves_splits_on_error(&self) -> bool {
        match self {
            Self::InputDisappeared | Self::StdoutWriteError(_) => true,
            Self::DeferredOutput { source, .. } => source.preserves_splits_on_error(),
            _ => false,
        }
    }

    pub fn deferred_stdout_lines(&self) -> Option<impl Iterator<Item = &str>> {
        match self {
            Self::DeferredOutput { stdout_lines, .. } => {
                Some(stdout_lines.iter().map(String::as_str))
            }
            _ => None,
        }
    }

    pub fn emit_deferred_stdout(&self) -> io::Result<()> {
        if let Some(stdout_lines) = self.deferred_stdout_lines() {
            for line in stdout_lines {
                super::write_stdout_line(line)?;
            }
        }
        Ok(())
    }

    pub fn replace_source_preserving_deferred(self, new_source: Self) -> Self {
        match self {
            Self::DeferredOutput { stdout_lines, .. } => {
                Self::with_deferred_stdout(new_source, stdout_lines)
            }
            _ => new_source,
        }
    }
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
    #[cfg(test)]
    use ctcore::ct_display::locale_quote;

    #[test]
    fn test_csplit_error_io_error() {
        use crate::csplit_error::CsplitError;
        let error = std::io::Error::other("IO error");
        let csplit_error = CsplitError::from(error);

        assert_eq!(csplit_error.to_string(), "IO error: IO error");
    }

    #[test]
    fn test_csplit_error_line_out_of_range() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::LineOutOfRange("file.txt".into());

        assert_eq!(
            error.to_string(),
            format!("{}: line number out of range", locale_quote("file.txt"))
        );
    }

    #[test]
    fn test_csplit_error_line_out_of_range_on_repetition() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::LineOutOfRangeOnRepetition("file.txt".into(), 3);

        assert_eq!(
            error.to_string(),
            format!(
                "{}: line number out of range on repetition 3",
                locale_quote("file.txt")
            )
        );
    }

    #[test]
    fn test_csplit_error_match_not_found() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::MatchNotFound("pattern".into());

        assert_eq!(
            error.to_string(),
            format!("{}: match not found", locale_quote("pattern"))
        );
    }

    #[test]
    fn test_csplit_error_match_not_found_on_repetition() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::MatchNotFoundOnRepetition("pattern".into(), 2);

        assert_eq!(
            error.to_string(),
            format!(
                "{}: match not found on repetition 2",
                locale_quote("pattern")
            )
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

        assert_eq!(
            error.to_string(),
            format!("{}: invalid pattern", locale_quote("invalid"))
        );
    }

    #[test]
    fn test_csplit_error_invalid_number() {
        use crate::csplit_error::CsplitError;
        let error = CsplitError::InvalidNumber("123".into());

        assert_eq!(
            error.to_string(),
            format!(
                "invalid number: {}",
                ctcore::ct_display::locale_quote("123")
            )
        );
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
}
