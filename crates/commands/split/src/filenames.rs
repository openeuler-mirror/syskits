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
//!
//! The [`FilenameIterator`] yields filenames for use with ``split``.
//!
//! # Examples
//!
//! Create filenames of the form `chunk_??.txt`:
//!
//! ```rust,ignore
//! use crate::filenames::FilenameIterator;
//! use crate::filenames::SuffixType;
//!
//! let prefix = "chunk_".to_string();
//! let suffix = Suffix {
//!     stype: SuffixType::Alphabetic,
//!     length: 2,
//!     start: 0,
//!     auto_widening: true,
//!     additional: ".txt".to_string(),
//! };
//! let it = FilenameIterator::new(prefix, suffix);
//!
//! assert_eq!(it.next().unwrap(), "chunk_aa.txt");
//! assert_eq!(it.next().unwrap(), "chunk_ab.txt");
//! assert_eq!(it.next().unwrap(), "chunk_ac.txt");
//! ```

use crate::number::DynamicWidthNumber;
use crate::number::Number;
use crate::number::NumberFixedWidthNumber;
use crate::strategy::Strategy;
use crate::{
    OPT_ADDITIONAL_SUFFIX, OPT_HEX_SUFFIXES, OPT_HEX_SUFFIXES_SHORT, OPT_NUMERIC_SUFFIXES,
    OPT_NUMERIC_SUFFIXES_SHORT, OPT_SUFFIX_LENGTH,
};
use clap::ArgMatches;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::is_separator;

/// The format to use for suffixes in the filename for each output chunk.
#[derive(Clone, Copy)]
pub enum FilenameSuffixType {
    /// Lowercase ASCII alphabetic characters.
    Alphabetic,

    /// Decimal numbers.
    Decimal,

    /// Hexadecimal numbers.
    Hexadecimal,
}

impl FilenameSuffixType {
    /// The radix to use when representing the suffix string as digits.
    pub fn radix(&self) -> u8 {
        match self {
            Self::Alphabetic => 26,
            Self::Decimal => 10,
            Self::Hexadecimal => 16,
        }
    }
}

/// Filename suffix parameters
#[derive(Clone)]
pub struct FilenameSuffix {
    stype: FilenameSuffixType,
    length: usize,
    start: usize,
    /// Digits for a suffix start that exceeds the host `usize` range.
    start_digits: Option<Vec<u8>>,
    auto_widening: bool,
    additional: String,
}

/// An error when parsing suffix parameters from command-line arguments.
#[derive(Debug)]
pub enum FilenameSuffixError {
    /// Invalid suffix length parameter.
    NotParsable(String),

    /// Invalid decimal or hexadecimal suffix start value.
    InvalidStartValue { value: String, hexadecimal: bool },

    /// Suffix contains a directory separator, which is not allowed.
    ContainsSeparator(String),

    /// Suffix is not large enough to split into specified chunks
    TooSmall(usize),
}

impl fmt::Display for FilenameSuffixError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::NotParsable(s) => write!(f, "invalid suffix length: {}", s.quote()),
            Self::InvalidStartValue { value, hexadecimal } => write!(
                f,
                "{}: invalid start value for {} suffix",
                value.quote(),
                if *hexadecimal {
                    "hexadecimal"
                } else {
                    "numerical"
                }
            ),
            Self::TooSmall(i) => write!(f, "the suffix length needs to be at least {i}"),
            Self::ContainsSeparator(s) => write!(
                f,
                "invalid suffix {}, contains directory separator",
                s.quote()
            ),
        }
    }
}

impl FilenameSuffix {
    fn parse_start_value(
        value: &str,
        hexadecimal: bool,
    ) -> Result<(usize, Option<Vec<u8>>), FilenameSuffixError> {
        let valid = if hexadecimal {
            value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        } else {
            value.bytes().all(|byte| byte.is_ascii_digit())
        };
        if !valid {
            return Err(FilenameSuffixError::InvalidStartValue {
                value: value.to_string(),
                hexadecimal,
            });
        }

        let normalized = value.trim_start_matches('0');
        if normalized.is_empty() {
            return Ok((0, None));
        }

        let radix = if hexadecimal { 16 } else { 10 };
        match usize::from_str_radix(normalized, radix) {
            Ok(start) => Ok((start, None)),
            Err(_) => Ok((
                0,
                Some(
                    normalized
                        .bytes()
                        .map(|byte| match byte {
                            b'0'..=b'9' => byte - b'0',
                            b'a'..=b'f' => byte - b'a' + 10,
                            _ => unreachable!("suffix start was validated before conversion"),
                        })
                        .collect(),
                ),
            )),
        }
    }

    /// Parse the suffix type, start, length and additional suffix from the command-line arguments
    /// as well process suffix length auto-widening and auto-width scenarios
    ///
    /// Suffix auto-widening: Determine if the output file names suffix is allowed to dynamically auto-widen,
    /// i.e. change (increase) suffix length dynamically as more files need to be written into.
    /// Suffix length auto-widening rules are (in the order they are applied):
    /// - ON by default
    /// - OFF when suffix start N is specified via long option with a value
    ///   `--numeric-suffixes=N` or `--hex-suffixes=N`
    /// - OFF when suffix length N is specified, except for N=0 (see edge cases below)
    ///   `-a N` or `--suffix-length=N`
    /// - OFF if suffix length is auto pre-calculated (auto-width)
    ///
    /// Suffix auto-width: Determine if the the output file names suffix length should be automatically pre-calculated
    /// based on number of files that need to written into, having number of files known upfront
    /// Suffix length auto pre-calculation rules:
    /// - Pre-calculate new suffix length when `-n`/`--number` option (N, K/N, l/N, l/K/N, r/N, r/K/N)
    ///   is used, where N is number of chunks = number of files to write into
    ///   and suffix start < N number of files
    ///   as in `split --numeric-suffixes=1 --number=r/100 file`
    /// - Do NOT pre-calculate new suffix length otherwise, i.e. when
    ///   suffix start >= N number of files
    ///   as in `split --numeric-suffixes=100 --number=r/100 file`
    ///   OR when suffix length N is specified, except for N=0 (see edge cases below)
    ///   `-a N` or `--suffix-length=N`
    ///
    /// Edge case:
    /// - If suffix length is specified as 0 in a command line,
    ///   first apply auto-width calculations and if still 0
    ///   set it to default value.
    ///   Do NOT change auto-widening value
    ///
    /**
     * 根据命令行参数和策略创建一个具有指定文件名后缀类型、起始值、长度和自动扩展设置的对象。
     *
     * @param args_match 命令行参数匹配结果，用于获取用户指定的后缀类型、起始值、长度等选项。
     * @param strategy 用于生成文件名后缀的策略，可以影响后缀的计算和处理方式。
     * @return 返回一个表示文件名后缀配置的结果，如果处理过程中遇到错误，则返回相应的错误信息。
     */
    pub fn from(args_match: &ArgMatches, strategy: &Strategy) -> Result<Self, FilenameSuffixError> {
        let stype: FilenameSuffixType;

        // 初始化默认值
        let mut start = 0;
        let mut start_digits = None;
        let mut auto_widening = true;
        let default_length: usize = 2;

        // 根据命令行参数确定后缀类型和起始值
        match (
            args_match.contains_id(OPT_NUMERIC_SUFFIXES),
            args_match.contains_id(OPT_HEX_SUFFIXES),
            args_match.get_flag(OPT_NUMERIC_SUFFIXES_SHORT),
            args_match.get_flag(OPT_HEX_SUFFIXES_SHORT),
        ) {
            (true, _, _, _) => {
                stype = FilenameSuffixType::Decimal;
                if let Some(opt) = args_match.get_one::<String>(OPT_NUMERIC_SUFFIXES) {
                    (start, start_digits) = Self::parse_start_value(opt, false)?;
                    auto_widening = false;
                }
            }
            (_, true, _, _) => {
                stype = FilenameSuffixType::Hexadecimal;
                if let Some(opt) = args_match.get_one::<String>(OPT_HEX_SUFFIXES) {
                    (start, start_digits) = Self::parse_start_value(opt, true)?;
                    auto_widening = false;
                }
            }
            (_, _, true, _) => stype = FilenameSuffixType::Decimal, // 短数字后缀 '-d'
            (_, _, _, true) => stype = FilenameSuffixType::Hexadecimal, // 短十六进制后缀 '-x'
            _ => stype = FilenameSuffixType::Alphabetic, // 未指定数字/十六进制后缀，使用默认字母后缀
        }

        // 获取后缀长度及其是否通过命令行选项指定
        let (mut length, is_length_cmd_opt) =
            if let Some(v) = args_match.get_one::<String>(OPT_SUFFIX_LENGTH) {
                (
                    v.trim_start_matches(|c: char| c.is_ascii_whitespace())
                        .parse::<usize>()
                        .map_err(|_| FilenameSuffixError::NotParsable(v.to_string()))?,
                    true,
                )
            } else {
                (default_length, false)
            };

        // 如果命令行指定了后缀长度且大于0，则禁用自动宽度扩展
        if is_length_cmd_opt && length > 0 {
            auto_widening = false;
        }

        // 如有必要，自动预先计算新的后缀长度（自动宽度）
        if let Strategy::Number(number_type) = strategy {
            let chunks = number_type.num_chunks();
            let required_length = ((start as u64 + chunks) as f64)
                .log(stype.radix() as f64)
                .ceil() as usize;

            if (start as u64) < chunks && !(is_length_cmd_opt && length > 0) {
                auto_widening = false;

                if length < required_length {
                    length = required_length;
                }
            }

            if length < required_length {
                return Err(FilenameSuffixError::TooSmall(required_length));
            }
        }

        // 检查命令行指定的后缀长度为0的边界情况，将其设置为默认值
        if is_length_cmd_opt && length == 0 {
            length = default_length;
        }

        // 获取额外的后缀信息，并检查其中是否包含分隔符
        let additional = Self::get_additional(args_match);
        if additional.chars().any(is_separator) {
            return Err(FilenameSuffixError::ContainsSeparator(additional));
        }

        // 创建并返回文件名后缀配置结果
        let result = Self {
            stype,
            length,
            start,
            start_digits,
            auto_widening,
            additional,
        };

        Ok(result)
    }

    fn get_additional(matches: &ArgMatches) -> String {
        matches
            .get_one::<String>(OPT_ADDITIONAL_SUFFIX)
            .unwrap()
            .to_string()
    }
}

/// Compute filenames from a given index.
///
/// This iterator yields filenames for use with ``split``.
///
/// The `prefix` is prepended to each filename and the
/// `suffix.additional` is appended to each filename.
///
/// If `suffix.auto_widening` is true, then the variable portion of the filename
/// that identifies the current chunk will have a dynamically
/// increasing width. If `suffix.auto_widening` is false, then
/// the variable portion of the filename will always be exactly `suffix.length`
/// width in characters. In that case, after the iterator yields each
/// string of that width, the iterator is exhausted.
///
/// Finally, `suffix.stype` controls which type of suffix to produce,
/// alphabetic or numeric.
///
/// # Examples
///
/// Create filenames of the form `chunk_??.txt`, where the `?`
/// characters are lowercase ASCII alphabetic characters:
///
/// ```rust,ignore
/// use crate::filenames::FilenameIterator;
/// use crate::filenames::SuffixType;
///
/// let prefix = "chunk_".to_string();
/// let suffix = Suffix {
///     stype: SuffixType::Alphabetic,
///     length: 2,
///     start: 0,
///     auto_widening: true,
///     additional: ".txt".to_string(),
/// };
/// let it = FilenameIterator::new(prefix, suffix);
///
/// assert_eq!(it.next().unwrap(), "chunk_aa.txt");
/// assert_eq!(it.next().unwrap(), "chunk_ab.txt");
/// assert_eq!(it.next().unwrap(), "chunk_ac.txt");
/// ```
///
/// For decimal numeric filenames, use `SuffixType::Decimal`:
///
/// ```rust,ignore
/// use crate::filenames::FilenameIterator;
/// use crate::filenames::SuffixType;
///
/// let prefix = "chunk_".to_string();
/// let suffix = Suffix {
///     stype: SuffixType::Decimal,
///     length: 2,
///     start: 0,
///     auto_widening: true,
///     additional: ".txt".to_string(),
/// };
/// let it = FilenameIterator::new(prefix, suffix);
///
/// assert_eq!(it.next().unwrap(), "chunk_00.txt");
/// assert_eq!(it.next().unwrap(), "chunk_01.txt");
/// assert_eq!(it.next().unwrap(), "chunk_02.txt");
/// ```
pub struct FilenameIterator {
    prefix: OsString,
    additional_suffix: String,
    number: Number,
    first_iteration: bool,
}

impl FilenameIterator {
    pub fn new(
        file_prefix: impl AsRef<OsStr>,
        filename_suffix: &FilenameSuffix,
    ) -> CTResult<FilenameIterator> {
        let radix_size = filename_suffix.stype.radix();
        let file_suffix_number_size = if filename_suffix.auto_widening {
            Number::DynamicWidth(DynamicWidthNumber::new(radix_size, filename_suffix.start))
        } else {
            let fixed_width_number = if let Some(start_digits) = &filename_suffix.start_digits {
                NumberFixedWidthNumber::from_digits(
                    radix_size,
                    filename_suffix.length,
                    start_digits,
                )
            } else {
                NumberFixedWidthNumber::new(
                    radix_size,
                    filename_suffix.length,
                    filename_suffix.start,
                )
            };
            Number::FixedWidth(fixed_width_number.map_err(|_| {
                CtSimpleError::new(
                    1,
                    "numerical suffix start value is too large for the suffix length",
                )
            })?)
        };
        Ok(FilenameIterator {
            prefix: file_prefix.as_ref().to_os_string(),
            additional_suffix: filename_suffix.additional.clone(),
            number: file_suffix_number_size,
            first_iteration: true,
        })
    }
}

impl Iterator for FilenameIterator {
    type Item = OsString;

    fn next(&mut self) -> Option<Self::Item> {
        if self.first_iteration {
            self.first_iteration = false;
        } else {
            self.number.number_increment().ok()?;
        }
        let mut filename = self.prefix.to_os_string();
        filename.push(self.number.to_string());
        filename.push(self.additional_suffix.as_str());
        Some(filename)
    }
}

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::ct_app;
    use crate::filenames::FilenameSuffix;
    use crate::filenames::FilenameSuffixType;
    use crate::filenames::{FilenameIterator, FilenameSuffixError};
    use crate::strategy::Strategy;
    use std::ffi::OsString;

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_initial_three_iterations() {
            let suffix = FilenameSuffix {
                stype: FilenameSuffixType::Alphabetic,
                length: 2,
                start: 0,
                start_digits: None,
                auto_widening: false,
                additional: ".txt".to_string(),
            };
            let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
            assert_eq!(it.next().unwrap(), "chunk_aa.txt");
            assert_eq!(it.next().unwrap(), "chunk_ab.txt");
            assert_eq!(it.next().unwrap(), "chunk_ac.txt");
        }

        #[test]
        fn test_skip_to_last_iteration() {
            let suffix = FilenameSuffix {
                stype: FilenameSuffixType::Alphabetic,
                length: 2,
                start: 0,
                start_digits: None,
                auto_widening: false,
                additional: ".txt".to_string(),
            };
            let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
            assert_eq!(it.nth(26 * 26 - 1).unwrap(), "chunk_zz.txt");
        }

        #[test]
        fn test_end_of_iteration() {
            let suffix = FilenameSuffix {
                stype: FilenameSuffixType::Alphabetic,
                length: 2,
                start: 0,
                start_digits: None,
                auto_widening: false,
                additional: ".txt".to_string(),
            };
            let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
            it.nth(26 * 26 - 1).unwrap();
            assert_eq!(it.next(), None);
        }
        #[test]
        fn test_filename_iterator_alphabetic_dynamic_width() {
            let suffix = FilenameSuffix {
                stype: FilenameSuffixType::Alphabetic,
                length: 2,
                start: 0,
                start_digits: None,
                auto_widening: true,
                additional: ".txt".to_string(),
            };
            let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
            assert_eq!(it.next().unwrap(), "chunk_aa.txt");
        }
        #[test]
        fn test_filename_iterator_numeric_dynamic_width() {
            let suffix = FilenameSuffix {
                stype: FilenameSuffixType::Decimal,
                length: 2,
                start: 0,
                start_digits: None,
                auto_widening: true,
                additional: ".txt".to_string(),
            };
            let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
            assert_eq!(it.next().unwrap(), "chunk_00.txt");
        }
        #[test]
        fn test_filename_iterator_numeric_fixed_width() {
            let suffix = FilenameSuffix {
                stype: FilenameSuffixType::Decimal,
                length: 2,
                start: 0,
                start_digits: None,
                auto_widening: false,
                additional: ".txt".to_string(),
            };
            let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
            assert_eq!(it.next().unwrap(), "chunk_00.txt");
        }
        #[test]
        fn test_filename_iterator_numeric_fixed_width_start() {
            let suffix = FilenameSuffix {
                stype: FilenameSuffixType::Decimal,
                length: 2,
                start: 10,
                start_digits: None,
                auto_widening: false,
                additional: ".txt".to_string(),
            };
            let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
            assert_eq!(it.next().unwrap(), "chunk_10.txt");
        }
    }

    #[test]
    fn test_initial_three_iterations_numeric() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Decimal,
            length: 2,
            start: 0,
            start_digits: None,
            auto_widening: false,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.next().unwrap(), "chunk_00.txt");
        assert_eq!(it.next().unwrap(), "chunk_01.txt");
        assert_eq!(it.next().unwrap(), "chunk_02.txt");
    }

    #[test]
    fn test_skip_to_last_iteration_numeric() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Decimal,
            length: 2,
            start: 0,
            start_digits: None,
            auto_widening: false,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.nth(10 * 10 - 1).unwrap(), "chunk_99.txt");
    }

    #[test]
    fn test_end_of_iteration_numeric() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Decimal,
            length: 2,
            start: 0,
            start_digits: None,
            auto_widening: false,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        it.nth(10 * 10 - 1).unwrap();
        assert_eq!(it.next(), None);
    }

    #[test]
    fn test_initial_three_iterations_alphabetic_dynamic() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Alphabetic,
            length: 2,
            start: 0,
            start_digits: None,
            auto_widening: true,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.next().unwrap(), "chunk_aa.txt");
        assert_eq!(it.next().unwrap(), "chunk_ab.txt");
        assert_eq!(it.next().unwrap(), "chunk_ac.txt");
    }

    #[test]
    fn test_skip_to_widened_iteration() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Alphabetic,
            length: 2,
            start: 0,
            start_digits: None,
            auto_widening: true,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.nth(26 * 25 - 1).unwrap(), "chunk_yz.txt");
        assert_eq!(it.next().unwrap(), "chunk_zaaa.txt");
    }

    #[test]
    fn test_next_widened_iteration() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Alphabetic,
            length: 2,
            start: 0,
            start_digits: None,
            auto_widening: true,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        it.nth(26 * 25 - 1).unwrap();
        assert_eq!(it.next().unwrap(), "chunk_zaaa.txt");
        assert_eq!(it.next().unwrap(), "chunk_zaab.txt");
    }

    #[test]
    fn test_initial_three_iterations_numeric_dynamic() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Decimal,
            length: 2,
            start: 0,
            start_digits: None,
            auto_widening: true,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.next().unwrap(), "chunk_00.txt");
        assert_eq!(it.next().unwrap(), "chunk_01.txt");
        assert_eq!(it.next().unwrap(), "chunk_02.txt");
    }

    #[test]
    fn test_skip_to_widened_iteration_numeric() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Decimal,
            length: 2,
            start: 0,
            start_digits: None,
            auto_widening: true,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.nth(10 * 9 - 1).unwrap(), "chunk_89.txt");
        assert_eq!(it.next().unwrap(), "chunk_9000.txt");
    }

    #[test]
    fn test_next_widened_iteration_numeric() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Decimal,
            length: 2,
            start: 0,
            start_digits: None,
            auto_widening: true,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        it.nth(10 * 9 - 1).unwrap();
        assert_eq!(it.next().unwrap(), "chunk_9000.txt");
        assert_eq!(it.next().unwrap(), "chunk_9001.txt");
    }
    #[test]
    fn test_filename_iterator_numeric_decimal() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Decimal,
            length: 2,
            start: 5,
            start_digits: None,
            auto_widening: true,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.next().unwrap(), "chunk_05.txt");
        assert_eq!(it.next().unwrap(), "chunk_06.txt");
        assert_eq!(it.next().unwrap(), "chunk_07.txt");
    }

    #[test]
    fn test_filename_iterator_numeric_hex() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Hexadecimal,
            length: 2,
            start: 9,
            start_digits: None,
            auto_widening: true,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.next().unwrap(), "chunk_09.txt");
        assert_eq!(it.next().unwrap(), "chunk_0a.txt");
        assert_eq!(it.next().unwrap(), "chunk_0b.txt");
    }

    #[test]
    fn test_filename_iterator_numeric_octal() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Alphabetic,
            length: 2,
            start: 0,
            start_digits: None,
            auto_widening: true,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.next().unwrap(), "chunk_aa.txt");
        assert_eq!(it.next().unwrap(), "chunk_ab.txt");
        assert_eq!(it.next().unwrap(), "chunk_ac.txt");
    }

    #[test]
    fn test_filename_iterator_numeric_reached_max_value() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Decimal,
            length: 3,
            start: 999,
            start_digits: None,
            auto_widening: false,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.next().unwrap(), "chunk_999.txt");
        assert!(it.next().is_none());
    }

    #[test]
    fn test_filename_iterator_numeric_start_out_of_range() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Decimal,
            length: 3,
            start: 1000,
            start_digits: None,
            auto_widening: false,
            additional: ".txt".to_string(),
        };
        let it = FilenameIterator::new("chunk_", &suffix);
        assert!(it.is_err());
    }

    #[test]
    fn test_filename_iterator_hexadecimal_reached_max_value() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Hexadecimal,
            length: 3,
            start: 0xfff,
            start_digits: None,
            auto_widening: false,
            additional: ".txt".to_string(),
        };
        let mut it = FilenameIterator::new("chunk_", &suffix).unwrap();
        assert_eq!(it.next().unwrap(), "chunk_fff.txt");
        assert!(it.next().is_none());
    }

    #[test]
    fn test_filename_iterator_hexadecimal_start_out_of_range() {
        let suffix = FilenameSuffix {
            stype: FilenameSuffixType::Hexadecimal,
            length: 3,
            start: 0x1000,
            start_digits: None,
            auto_widening: false,
            additional: ".txt".to_string(),
        };
        let it = FilenameIterator::new("chunk_", &suffix);
        assert!(it.is_err());
    }

    #[test]
    fn test_alphabetic_radix() {
        assert_eq!(FilenameSuffixType::Alphabetic.radix(), 26);
    }

    #[test]
    fn test_decimal_radix() {
        assert_eq!(FilenameSuffixType::Decimal.radix(), 10);
    }

    #[test]
    fn test_hexadecimal_radix() {
        assert_eq!(FilenameSuffixType::Hexadecimal.radix(), 16);
    }

    #[test]
    fn test_suffix_length_accepts_leading_ascii_whitespace() {
        let matches = ct_app()
            .try_get_matches_from(["split", "-a", "\t1", "-b", "1"])
            .expect("parse split arguments");
        let strategy = Strategy::from(&matches, &None).expect("parse byte strategy");

        assert!(FilenameSuffix::from(&matches, &strategy).is_ok());
    }

    #[test]
    fn test_suffix_start_accepts_empty_decimal_and_hexadecimal_values() {
        for option in ["--numeric-suffixes=", "--hex-suffixes="] {
            let matches = ct_app()
                .try_get_matches_from(["split", option, "-b", "1"])
                .expect("parse split arguments");
            let strategy = Strategy::from(&matches, &None).expect("parse byte strategy");
            let suffix = FilenameSuffix::from(&matches, &strategy)
                .expect("empty suffix start must mean zero");

            assert_eq!(suffix.start, 0);
        }
    }

    #[test]
    fn test_filename_iterator_accepts_suffix_start_beyond_usize() {
        let start = "18446744073709551616";
        let matches = ct_app()
            .try_get_matches_from([
                "split",
                "--numeric-suffixes=18446744073709551616",
                "-a",
                "30",
                "-b",
                "1",
            ])
            .expect("parse split arguments");
        let strategy = Strategy::from(&matches, &None).expect("parse byte strategy");
        let suffix = FilenameSuffix::from(&matches, &strategy)
            .expect("large numerical suffix start must be accepted");
        let mut files = FilenameIterator::new("chunk-", &suffix)
            .expect("large numerical suffix must fit the selected width");

        assert_eq!(
            files.next().expect("first output name"),
            OsString::from(format!("chunk-{start:0>30}"))
        );
        assert_eq!(
            files.next().expect("second output name"),
            OsString::from("chunk-000000000018446744073709551617")
        );
    }

    #[test]
    fn test_suffix_start_rejects_non_gnu_digit_alphabet() {
        for (option, expected) in [
            (
                "--numeric-suffixes=+1",
                "'+1': invalid start value for numerical suffix",
            ),
            (
                "--hex-suffixes=A",
                "'A': invalid start value for hexadecimal suffix",
            ),
        ] {
            let matches = ct_app()
                .try_get_matches_from(["split", option, "-b", "1"])
                .expect("parse split arguments");
            let strategy = Strategy::from(&matches, &None).expect("parse byte strategy");
            let error = match FilenameSuffix::from(&matches, &strategy) {
                Err(error) => error,
                Ok(_) => panic!("non-GNU suffix start alphabet must be rejected"),
            };

            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn test_suffix_error_not_parsable_display() {
        let error = FilenameSuffixError::NotParsable("123".to_string());
        assert_eq!("invalid suffix length: '123'", format!("{error}"));
    }

    #[test]
    fn test_suffix_error_too_small_display() {
        let error = FilenameSuffixError::TooSmall(5);
        assert_eq!(
            "the suffix length needs to be at least 5",
            format!("{error}")
        );
    }

    #[test]
    fn test_suffix_error_contains_separator_display() {
        let error = FilenameSuffixError::ContainsSeparator("/".to_string());
        assert_eq!(
            "invalid suffix '/', contains directory separator",
            format!("{error}")
        );
    }
    #[test]
    fn test_suffix_error_contains_separator_debug() {
        let error = FilenameSuffixError::ContainsSeparator("/".to_string());
        assert_eq!("ContainsSeparator(\"/\")", format!("{error:?}"));
    }
    #[test]
    fn test_suffix_error_contains_separator_display_with_path() {
        let error = FilenameSuffixError::ContainsSeparator("/".to_string());
        assert_eq!(
            "invalid suffix '/', contains directory separator",
            format!("{error}")
        );
    }

    #[test]
    fn test_suffix_error_contains_separator_debug_with_path() {
        let error = FilenameSuffixError::ContainsSeparator("/".to_string());
        assert_eq!("ContainsSeparator(\"/\")", format!("{error:?}"));
    }
}
