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
//! based on the command line options

use crate::{OPT_BYTES, OPT_LINE_BYTES, OPT_LINES, OPT_NUMBER};
use clap::{ArgMatches, parser::ValueSource};
use ctcore::{
    ct_display::Quotable,
    ct_parse_size::{ParseSizeError, parse_size_u64_max},
};
use std::fmt;

struct ParsedIntmax {
    value: i64,
    end: usize,
    overflow: bool,
}

/// Parse the numeric prefix with the subset of `strtoimax` behavior used by
/// GNU split's `parse_chunk`: leading ASCII whitespace and an optional sign
/// are accepted, while overflow saturates to the matching `intmax_t` bound.
fn parse_intmax_prefix(input: &str) -> Option<ParsedIntmax> {
    let bytes = input.as_bytes();
    let mut start = 0;
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }

    let (negative, digits_start) = match bytes.get(start) {
        Some(b'-') => (true, start + 1),
        Some(b'+') => (false, start + 1),
        _ => (false, start),
    };
    let mut end = digits_start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == digits_start {
        return None;
    }

    let limit = if negative {
        i64::MAX as u64 + 1
    } else {
        i64::MAX as u64
    };
    let mut magnitude = 0_u64;
    let mut overflow = false;
    for byte in &bytes[digits_start..end] {
        let digit = u64::from(byte - b'0');
        if magnitude > (limit - digit) / 10 {
            magnitude = limit;
            overflow = true;
        } else if !overflow {
            magnitude = magnitude * 10 + digit;
        }
    }

    let value = if negative {
        if magnitude == i64::MAX as u64 + 1 {
            i64::MIN
        } else {
            -(magnitude as i64)
        }
    } else {
        magnitude as i64
    };
    Some(ParsedIntmax {
        value,
        end,
        overflow,
    })
}

fn parse_chunk_count(input: &str) -> Option<u64> {
    let parsed = parse_intmax_prefix(input)?;
    (parsed.end == input.len() && parsed.value > 0).then_some(parsed.value as u64)
}

fn parse_number_chunks(input: &str) -> Result<(Option<u64>, u64), StrategyNumberTypeError> {
    let input = input.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let parsed = parse_intmax_prefix(input)
        .ok_or_else(|| StrategyNumberTypeError::NumberOfChunks(input.to_string()))?;

    if !parsed.overflow && input.as_bytes().get(parsed.end) == Some(&b'/') {
        let chunk_number = parsed.value;
        let num_chunks_text = &input[parsed.end + 1..];
        let num_chunks = parse_chunk_count(num_chunks_text)
            .ok_or_else(|| StrategyNumberTypeError::NumberOfChunks(num_chunks_text.to_string()))?;
        if chunk_number <= 0 || chunk_number as u64 > num_chunks {
            return Err(StrategyNumberTypeError::ChunkNumber(
                input[..parsed.end].to_string(),
            ));
        }
        return Ok((Some(chunk_number as u64), num_chunks));
    }

    if parsed.end == input.len() && parsed.value > 0 {
        Ok((None, parsed.value as u64))
    } else {
        Err(StrategyNumberTypeError::NumberOfChunks(input.to_string()))
    }
}

/// Sub-strategy of the [`Strategy::Number`]
/// Splitting a file into a specific number of chunks.
#[derive(Debug, PartialEq)]
pub enum StrategyNumberType {
    /// Split into a specific number of chunks by byte.
    Bytes(u64),

    /// Split into a specific number of chunks by byte
    /// but output only the *k*th chunk.
    KthBytes(u64, u64),

    /// Split into a specific number of chunks by line (approximately).
    Lines(u64),

    /// Split into a specific number of chunks by line
    /// (approximately), but output only the *k*th chunk.
    KthLines(u64, u64),

    /// Assign lines via round-robin to the specified number of output chunks.
    RoundRobin(u64),

    /// Assign lines via round-robin to the specified number of output
    /// chunks, but output only the *k*th chunk.
    KthRoundRobin(u64, u64),
}

impl StrategyNumberType {
    /// The number of chunks for this number type.
    pub fn num_chunks(&self) -> u64 {
        match self {
            Self::Bytes(n) => *n,
            Self::KthBytes(_, n) => *n,
            Self::Lines(n) => *n,
            Self::KthLines(_, n) => *n,
            Self::RoundRobin(n) => *n,
            Self::KthRoundRobin(_, n) => *n,
        }
    }
}

/// An error due to an invalid parameter to the `-n` command-line option.
#[derive(Debug, PartialEq)]
pub enum StrategyNumberTypeError {
    /// The number of chunks was invalid.
    ///
    /// This can happen if the value of `N` in any of the following
    /// command-line options is not a positive integer:
    ///
    /// ```ignore
    /// -n N
    /// -n K/N
    /// -n l/N
    /// -n l/K/N
    /// -n r/N
    /// -n r/K/N
    /// ```
    NumberOfChunks(String),

    /// The chunk number was invalid.
    ///
    /// This can happen if the value of `K` in any of the following
    /// command-line options is not a positive integer
    /// or if `K` is 0
    /// or if `K` is greater than `N`:
    ///
    /// ```ignore
    /// -n K/N
    /// -n l/K/N
    /// -n r/K/N
    /// ```
    ChunkNumber(String),
}

impl StrategyNumberType {
    /// Parse a `NumberType` from a string.
    ///
    /// The following strings are valid arguments:
    ///
    /// ```ignore
    /// "N"
    /// "K/N"
    /// "l/N"
    /// "l/K/N"
    /// "r/N"
    /// "r/K/N"
    /// ```
    ///
    /// The `N` represents the number of chunks and the `K` represents
    /// a chunk number.
    ///
    /// # Errors
    ///
    /// If the string is not one of the valid number types,
    /// if `K` is not a nonnegative integer,
    /// or if `K` is 0,
    /// or if `N` is not a positive integer,
    /// or if `K` is greater than `N`
    /// then this function returns [`StrategyNumberTypeError`].
    fn from(s: &str) -> Result<Self, StrategyNumberTypeError> {
        let s = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
        let (mode, chunks) = if let Some(chunks) = s.strip_prefix("l/") {
            ('l', chunks)
        } else if let Some(chunks) = s.strip_prefix("r/") {
            ('r', chunks)
        } else {
            ('b', s)
        };
        let (chunk_number, num_chunks) = parse_number_chunks(chunks)?;

        match (mode, chunk_number) {
            ('b', None) => Ok(Self::Bytes(num_chunks)),
            ('b', Some(chunk_number)) => Ok(Self::KthBytes(chunk_number, num_chunks)),
            ('l', None) => Ok(Self::Lines(num_chunks)),
            ('l', Some(chunk_number)) => Ok(Self::KthLines(chunk_number, num_chunks)),
            ('r', None) => Ok(Self::RoundRobin(num_chunks)),
            ('r', Some(chunk_number)) => Ok(Self::KthRoundRobin(chunk_number, num_chunks)),
            _ => unreachable!("split number mode is one of b, l, or r"),
        }
    }
}

/// The strategy for breaking up the input file into chunks.
pub enum Strategy {
    /// Each chunk has the specified number of lines.
    Lines(u64),

    /// Each chunk has the specified number of bytes.
    Bytes(u64),

    /// Each chunk has as many lines as possible without exceeding the
    /// specified number of bytes.
    LineBytes(u64),

    /// Split the file into this many chunks.
    ///
    /// There are several sub-strategies available, as defined by
    /// [`StrategyNumberType`].
    Number(StrategyNumberType),
}

/// An error when parsing a chunking strategy from command-line arguments.
#[derive(Debug)]
pub enum StrategyError {
    /// Invalid number of lines.
    Lines(ParseSizeError),

    /// Invalid number of bytes.
    Bytes(ParseSizeError),

    /// Invalid number type.
    NumberType(StrategyNumberTypeError),

    /// Multiple chunking strategies were specified (but only one should be).
    MultipleWays,
}

impl fmt::Display for StrategyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Lines(e) => write!(f, "invalid number of lines: {e}"),
            Self::Bytes(e) => write!(f, "invalid number of bytes: {e}"),
            Self::NumberType(StrategyNumberTypeError::NumberOfChunks(s)) => {
                write!(f, "invalid number of chunks: {}", s.quote())
            }
            Self::NumberType(StrategyNumberTypeError::ChunkNumber(s)) => {
                write!(f, "invalid chunk number: {}", s.quote())
            }
            Self::MultipleWays => write!(f, "cannot split in more than one way"),
        }
    }
}

impl Strategy {
    /// Parse a strategy from the command-line arguments.
    pub fn from(
        args_match: &ArgMatches,
        obs_lines: &Option<String>,
    ) -> Result<Self, StrategyError> {
        fn get_and_parse(
            args_match: &ArgMatches,
            opt: &str,
            strategy: fn(u64) -> Strategy,
            error: fn(ParseSizeError) -> StrategyError,
        ) -> Result<Strategy, StrategyError> {
            let s = args_match.get_one::<String>(opt).unwrap();
            let n = parse_size_u64_max(s).map_err(error)?;
            if n > 0 {
                Ok(strategy(n))
            } else {
                Err(error(ParseSizeError::ParseFailure(format!(
                    "{}",
                    s.quote()
                ))))
            }
        }

        let strategy_options = [OPT_LINES, OPT_BYTES, OPT_LINE_BYTES, OPT_NUMBER];
        let strategy_occurrences = strategy_options
            .iter()
            .map(|option| {
                if args_match.value_source(option) == Some(ValueSource::CommandLine) {
                    args_match
                        .get_many::<String>(option)
                        .map(Iterator::count)
                        .unwrap_or(0)
                } else {
                    0
                }
            })
            .sum::<usize>();

        if obs_lines.is_none() && strategy_occurrences > 1 {
            let first_strategy = strategy_options
                .iter()
                .filter_map(|option| {
                    (args_match.value_source(option) == Some(ValueSource::CommandLine))
                        .then(|| {
                            args_match
                                .indices_of(option)
                                .and_then(|mut indices| indices.next())
                                .map(|index| (*option, index))
                        })
                        .flatten()
                })
                .min_by_key(|(_, index)| *index)
                .expect("at least one split strategy argument");

            if first_strategy.0 == OPT_LINES {
                let _ = get_and_parse(args_match, OPT_LINES, Self::Lines, StrategyError::Lines)?;
            } else if first_strategy.0 == OPT_BYTES {
                let _ = get_and_parse(args_match, OPT_BYTES, Self::Bytes, StrategyError::Bytes)?;
            } else if first_strategy.0 == OPT_LINE_BYTES {
                let _ = get_and_parse(
                    args_match,
                    OPT_LINE_BYTES,
                    Self::LineBytes,
                    StrategyError::Lines,
                )?;
            } else if first_strategy.0 == OPT_NUMBER {
                let value = args_match.get_one::<String>(OPT_NUMBER).unwrap();
                let _ = StrategyNumberType::from(value).map_err(StrategyError::NumberType)?;
            } else {
                unreachable!("split strategy option is known");
            }

            return Err(StrategyError::MultipleWays);
        }
        // 检查用户是否指定了超过一种策略。
        //
        // 注意：目前，由于“lines”值选项已弃用，此确切行为无法通过 overrides_with_all() 处理
        match (
            obs_lines,
            args_match.value_source(OPT_LINES) == Some(ValueSource::CommandLine),
            args_match.value_source(OPT_BYTES) == Some(ValueSource::CommandLine),
            args_match.value_source(OPT_LINE_BYTES) == Some(ValueSource::CommandLine),
            args_match.value_source(OPT_NUMBER) == Some(ValueSource::CommandLine),
        ) {
            (Some(v), false, false, false, false) => {
                let v = parse_size_u64_max(v).map_err(|_| {
                    StrategyError::Lines(ParseSizeError::ParseFailure(v.to_string()))
                })?;
                if v > 0 {
                    Ok(Self::Lines(v))
                } else {
                    Err(StrategyError::Lines(ParseSizeError::ParseFailure(
                        v.to_string(),
                    )))
                }
            }
            (None, false, false, false, false) => Ok(Self::Lines(1000)),
            (None, true, false, false, false) => {
                get_and_parse(args_match, OPT_LINES, Self::Lines, StrategyError::Lines)
            }
            (None, false, true, false, false) => {
                get_and_parse(args_match, OPT_BYTES, Self::Bytes, StrategyError::Bytes)
            }
            (None, false, false, true, false) => get_and_parse(
                args_match,
                OPT_LINE_BYTES,
                Self::LineBytes,
                StrategyError::Lines,
            ),
            (None, false, false, false, true) => {
                let s = args_match.get_one::<String>(OPT_NUMBER).unwrap();
                let number_type = StrategyNumberType::from(s).map_err(StrategyError::NumberType)?;
                Ok(Self::Number(number_type))
            }
            _ => Err(StrategyError::MultipleWays),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ct_app;
    use crate::strategy::Strategy;
    use crate::strategy::StrategyNumberType;
    use crate::strategy::StrategyNumberTypeError;

    #[test]
    fn test_line_bytes_zero_reports_invalid_number_of_lines() {
        let matches = ct_app()
            .try_get_matches_from(["split", "-C", "0"])
            .expect("parse split arguments");

        let error = match Strategy::from(&matches, &None) {
            Err(error) => error,
            Ok(_) => panic!("zero line bytes must be rejected"),
        };

        assert_eq!(error.to_string(), "invalid number of lines: '0'");
    }

    #[test]
    fn test_repeated_strategy_option_reports_multiple_ways() {
        let matches = ct_app()
            .try_get_matches_from(["split", "-l", "2", "-l", "3"])
            .expect("parse repeated strategy arguments");

        let error = match Strategy::from(&matches, &None) {
            Err(error) => error,
            Ok(_) => panic!("repeated split strategy must be rejected"),
        };

        assert!(matches!(error, super::StrategyError::MultipleWays));
    }

    #[test]
    fn test_invalid_first_repeated_strategy_operand_is_reported_before_conflict() {
        let matches = ct_app()
            .try_get_matches_from(["split", "-l", "not-a-number", "-l", "2"])
            .expect("parse repeated strategy arguments");

        let error = match Strategy::from(&matches, &None) {
            Err(error) => error,
            Ok(_) => panic!("invalid first split strategy operand must be rejected"),
        };

        assert_eq!(error.to_string(), "invalid number of lines: 'not-a-number'");
    }

    #[test]
    fn test_number_type_from_case_1() {
        let number_type = StrategyNumberType::from("123").unwrap();
        assert_eq!(number_type, StrategyNumberType::Bytes(123));
    }

    #[test]
    fn test_number_type_from_case_2() {
        let number_type = StrategyNumberType::from("l/123").unwrap();
        assert_eq!(number_type, StrategyNumberType::Lines(123));
    }

    #[test]
    fn test_number_type_from_case_3() {
        let number_type = StrategyNumberType::from("l/123/456").unwrap();
        assert_eq!(number_type, StrategyNumberType::KthLines(123, 456));
    }

    #[test]
    fn test_number_type_from_case_4() {
        let number_type = StrategyNumberType::from("r/123").unwrap();
        assert_eq!(number_type, StrategyNumberType::RoundRobin(123));
    }

    #[test]
    fn test_number_type_from_case_5() {
        let number_type = StrategyNumberType::from("r/123/456").unwrap();
        assert_eq!(number_type, StrategyNumberType::KthRoundRobin(123, 456));
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_number_type_from_error_case_1() {
        assert_eq!(
            StrategyNumberType::from("xyz").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("xyz".to_string())
        );
    }
    #[test]
    fn test_number_type_from_error_case_() {
        assert_eq!(
            StrategyNumberType::from("l/xyz").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("xyz".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_2() {
        assert_eq!(
            StrategyNumberType::from("l/xyz").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("xyz".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_3() {
        assert_eq!(
            StrategyNumberType::from("l/123/xyz").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("xyz".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_4() {
        assert_eq!(
            StrategyNumberType::from("l/abc/456").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("abc/456".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_5() {
        assert_eq!(
            StrategyNumberType::from("l/456/123").unwrap_err(),
            StrategyNumberTypeError::ChunkNumber("456".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_6() {
        assert_eq!(
            StrategyNumberType::from("r/456/123").unwrap_err(),
            StrategyNumberTypeError::ChunkNumber("456".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_7() {
        assert_eq!(
            StrategyNumberType::from("456/123").unwrap_err(),
            StrategyNumberTypeError::ChunkNumber("456".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_8() {
        assert_eq!(
            StrategyNumberType::from("l/abc/xyz").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("abc/xyz".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_9() {
        assert_eq!(
            StrategyNumberType::from("r/xyz").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("xyz".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_10() {
        assert_eq!(
            StrategyNumberType::from("r/123/xyz").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("xyz".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_11() {
        assert_eq!(
            StrategyNumberType::from("r/abc/456").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("abc/456".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_12() {
        assert_eq!(
            StrategyNumberType::from("r/abc/xyz").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("abc/xyz".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_13() {
        assert_eq!(
            StrategyNumberType::from("r/abc/xyz").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("abc/xyz".to_string())
        );
    }

    #[test]
    fn test_number_type_from_error_case_14() {
        assert_eq!(
            StrategyNumberType::from("r/abc/xyz").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("abc/xyz".to_string())
        );
    }

    #[test]
    fn test_number_type_uses_gnu_decimal_chunk_syntax() {
        assert_eq!(
            StrategyNumberType::from("+1/1").unwrap(),
            StrategyNumberType::KthBytes(1, 1)
        );
        assert_eq!(
            StrategyNumberType::from(" 1/ 1").unwrap(),
            StrategyNumberType::KthBytes(1, 1)
        );
        assert_eq!(
            StrategyNumberType::from("l/1/+1").unwrap(),
            StrategyNumberType::KthLines(1, 1)
        );
        assert_eq!(
            StrategyNumberType::from("r/1/+1").unwrap(),
            StrategyNumberType::KthRoundRobin(1, 1)
        );
    }

    #[test]
    fn test_number_type_rejects_size_suffixes_with_gnu_error_scope() {
        assert_eq!(
            StrategyNumberType::from("1K/1K").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("1K/1K".to_string())
        );
        assert_eq!(
            StrategyNumberType::from("1/2K").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("2K".to_string())
        );
        assert_eq!(
            StrategyNumberType::from("l/1K/1K").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("1K/1K".to_string())
        );
    }

    #[test]
    fn test_number_type_rejects_zero_line_chunks() {
        assert_eq!(
            StrategyNumberType::from("l/0").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("0".to_string())
        );
    }

    #[test]
    fn test_number_type_rejects_zero_round_robin_chunks() {
        assert_eq!(
            StrategyNumberType::from("r/0").unwrap_err(),
            StrategyNumberTypeError::NumberOfChunks("0".to_string())
        );
    }

    #[test]
    fn test_number_type_rejects_zero_chunks_before_validating_kth_chunk() {
        for value in ["1/0", "l/1/0", "r/1/0"] {
            assert_eq!(
                StrategyNumberType::from(value).unwrap_err(),
                StrategyNumberTypeError::NumberOfChunks("0".to_string())
            );
        }
    }

    #[test]
    fn test_number_type_num_chunks_case_1() {
        let number_type = StrategyNumberType::from("123").unwrap();
        assert_eq!(number_type.num_chunks(), 123);
    }

    #[test]
    fn test_number_type_num_chunks_case_2() {
        let number_type = StrategyNumberType::from("123/456").unwrap();
        assert_eq!(number_type.num_chunks(), 456);
    }

    #[test]
    fn test_number_type_num_chunks_case_3() {
        let number_type = StrategyNumberType::from("l/123").unwrap();
        assert_eq!(number_type.num_chunks(), 123);
    }

    #[test]
    fn test_number_type_num_chunks_case_4() {
        let number_type = StrategyNumberType::from("l/123/456").unwrap();
        assert_eq!(number_type.num_chunks(), 456);
    }

    #[test]
    fn test_number_type_num_chunks_case_5() {
        let number_type = StrategyNumberType::from("r/123").unwrap();
        assert_eq!(number_type.num_chunks(), 123);
    }

    #[test]
    fn test_number_type_num_chunks_case_6() {
        let number_type = StrategyNumberType::from("r/123/456").unwrap();
        assert_eq!(number_type.num_chunks(), 456);
    }
}
