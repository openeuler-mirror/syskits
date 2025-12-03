/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use std::error::Error;
use std::fmt;
use std::num::IntErrorKind;

use crate::ct_display::Quotable;

/// Parser for sizes in SI or IEC units (multiples of 1000 or 1024 bytes).
///
/// The [`Parser::parse`] function performs the parse.
#[derive(Default)]
pub struct Parser<'parser> {
    /// Whether to allow empty numeric strings.
    pub no_empty_numeric: bool,
    /// Whether to treat the suffix "B" as meaning "bytes".
    pub capital_b_bytes: bool,
    /// Whether to treat "b" as a "byte count" instead of "block"
    pub b_byte_count: bool,
    /// Whitelist for the suffix
    pub allow_list: Option<&'parser [&'parser str]>,
    /// Default unit when no suffix is provided
    pub default_unit: Option<&'parser str>,
}

#[derive(PartialEq)]
enum NumberSystem {
    Decimal,
    Octal,
    Hexadecimal,
}

impl<'parser> Parser<'parser> {
    pub fn with_allow_list(&mut self, allow_list: &'parser [&str]) -> &mut Self {
        self.allow_list = Some(allow_list);
        self
    }

    pub fn with_default_unit(&mut self, default_unit: &'parser str) -> &mut Self {
        self.default_unit = Some(default_unit);
        self
    }

    pub fn with_b_byte_count(&mut self, value: bool) -> &mut Self {
        self.b_byte_count = value;
        self
    }

    pub fn with_allow_empty_numeric(&mut self, value: bool) -> &mut Self {
        self.no_empty_numeric = value;
        self
    }
    /// Parse a size string into a number of bytes.
    ///
    /// A size string comprises an integer and an optional unit. The unit
    /// may be K, M, G, T, P, E, Z, Y, R or Q (powers of 1024), or KB, MB,
    /// etc. (powers of 1000), or b which is 512.
    /// Binary prefixes can be used, too: KiB=K, MiB=M, and so on.
    ///
    /// # Errors
    ///
    /// Will return `ParseSizeError` if it's not possible to parse this
    /// string into a number, e.g. if the string does not begin with a
    /// numeral, or if the unit is not one of the supported units described
    /// in the preceding section.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ctcore::ct_parse_size::Parser;
    /// let parser = Parser {
    ///     default_unit: Some("M"),
    ///     ..Default::default()
    /// };
    /// assert_eq!(Ok(123 * 1024 * 1024), parser.parse("123M")); // M is 1024^2
    /// assert_eq!(Ok(123 * 1024 * 1024), parser.parse("123")); // default unit set to "M" on parser instance
    /// assert_eq!(Ok(9 * 1000), parser.parse("9kB")); // kB is 1000
    /// assert_eq!(Ok(2 * 1024), parser.parse("2K")); // K is 1024
    /// assert_eq!(Ok(44251 * 1024), parser.parse("0xACDBK")); // 0xACDB is 44251 in decimal
    /// ```
    pub fn parse(&self, size: &str) -> Result<u128, ParseSizeError> {
        if size.is_empty() {
            return Err(ParseSizeError::parse_failure(size));
        }

        let number_system = Self::determine_number_system(size);

        // Split the size argument into numeric and unit parts
        // For example, if the argument is "123K", the numeric part is "123", and
        // the unit is "K"
        let numeric_string: String = match number_system {
            NumberSystem::Hexadecimal => size
                .chars()
                .take(2)
                .chain(size.chars().skip(2).take_while(char::is_ascii_hexdigit))
                .collect(),
            _ => size.chars().take_while(char::is_ascii_digit).collect(),
        };
        let mut unit: &str = &size[numeric_string.len()..];

        if let Some(default_unit) = self.default_unit {
            // Check if `unit` is empty then assigns `default_unit` to `unit`
            if unit.is_empty() {
                unit = default_unit;
            }
        }

        // Check if `b` is a byte count and remove `b`
        if self.b_byte_count && unit.ends_with('b') {
            // If `unit` = 'b' then return error
            if numeric_string.is_empty() {
                return Err(ParseSizeError::parse_failure(size));
            }
            unit = &unit[0..unit.len() - 1];
        }

        if let Some(allow_list) = self.allow_list {
            // Check if `unit` appears in `allow_list`, if not return error
            if !allow_list.contains(&unit) && !unit.is_empty() {
                if numeric_string.is_empty() {
                    return Err(ParseSizeError::parse_failure(size));
                }
                return Err(ParseSizeError::invalid_suffix(size));
            }
        }

        // Compute the factor the unit represents.
        // empty string means the factor is 1.
        //
        // The lowercase "b" (used by `od`, `head`, `tail`, etc.) means
        // "block" and the Posix block size is 512. The uppercase "B"
        // means "byte".
        let base: u128;
        let exponent: u32;
        if unit.is_empty() || unit == "B" && self.capital_b_bytes {
            base = 1;
            exponent = 0;
        } else if unit == "b" {
            base = 512;
            exponent = 1;
        } else if unit == "KiB" || unit == "kiB" || unit == "K" || unit == "k" {
            base = 1024;
            exponent = 1;
        } else if unit == "MiB" || unit == "miB" || unit == "M" || unit == "m" {
            base = 1024;
            exponent = 2;
        } else if unit == "GiB" || unit == "giB" || unit == "G" || unit == "g" {
            base = 1024;
            exponent = 3;
        } else if unit == "TiB" || unit == "tiB" || unit == "T" || unit == "t" {
            base = 1024;
            exponent = 4;
        } else if unit == "PiB" || unit == "piB" || unit == "P" || unit == "p" {
            base = 1024;
            exponent = 5;
        } else if unit == "EiB" || unit == "eiB" || unit == "E" || unit == "e" {
            base = 1024;
            exponent = 6;
        } else if unit == "ZiB" || unit == "ziB" || unit == "Z" || unit == "z" {
            base = 1024;
            exponent = 7;
        } else if unit == "YiB" || unit == "yiB" || unit == "Y" || unit == "y" {
            base = 1024;
            exponent = 8;
        } else if unit == "RiB" || unit == "riB" || unit == "R" || unit == "r" {
            base = 1024;
            exponent = 9;
        } else if unit == "QiB" || unit == "qiB" || unit == "Q" || unit == "q" {
            base = 1024;
            exponent = 10;
        } else if unit == "KB" || unit == "kB" {
            base = 1000;
            exponent = 1;
        } else if unit == "MB" || unit == "mB" {
            base = 1000;
            exponent = 2;
        } else if unit == "GB" || unit == "gB" {
            base = 1000;
            exponent = 3;
        } else if unit == "TB" || unit == "tB" {
            base = 1000;
            exponent = 4;
        } else if unit == "PB" || unit == "pB" {
            base = 1000;
            exponent = 5;
        } else if unit == "EB" || unit == "eB" {
            base = 1000;
            exponent = 6;
        } else if unit == "ZB" || unit == "zB" {
            base = 1000;
            exponent = 7;
        } else if unit == "YB" || unit == "yB" {
            base = 1000;
            exponent = 8;
        } else if unit == "RB" || unit == "rB" {
            base = 1000;
            exponent = 9;
        } else if unit == "QB" || unit == "qB" {
            base = 1000;
            exponent = 10;
        } else if numeric_string.is_empty() {
            return Err(ParseSizeError::parse_failure(size));
        } else {
            return Err(ParseSizeError::invalid_suffix(size));
        }
        let factor = base.pow(exponent);

        // parse string into u128
        let number: u128;
        if number_system == NumberSystem::Decimal {
            if numeric_string.is_empty() && !self.no_empty_numeric {
                number = 1;
            } else {
                number = Self::parse_number(&numeric_string, 10, size)?;
            }
        } else if number_system == NumberSystem::Octal {
            let trimmed_string = numeric_string.trim_start_matches('0');
            number = Self::parse_number(trimmed_string, 8, size)?;
        } else if number_system == NumberSystem::Hexadecimal {
            let trimmed_string = numeric_string.trim_start_matches("0x");
            number = Self::parse_number(trimmed_string, 16, size)?;
        } else {
            // Handle other cases if needed
            unreachable!("Unexpected number system encountered");
        }
        // number
        //  .checked_mul(factor)
        // .ok_or_else(|| ParseSizeError::size_too_big(size))
        match number.checked_mul(factor) {
            Some(result) => Ok(result),
            None => Err(ParseSizeError::size_too_big(size)),
        }
    }

    /// Explicit u128 alias for `parse()`
    pub fn parse_u128(&self, size: &str) -> Result<u128, ParseSizeError> {
        self.parse(size)
    }

    /// Same as `parse()` but tries to return u64
    pub fn parse_u64(&self, size: &str) -> Result<u64, ParseSizeError> {
        match self.parse(size) {
            Ok(num_u128) => {
                let num_u64 = match u64::try_from(num_u128) {
                    Ok(n) => n,
                    Err(_) => return Err(ParseSizeError::size_too_big(size)),
                };
                Ok(num_u64)
            }
            Err(e) => Err(e),
        }
    }

    /// Same as `parse_u64()`, except returns `u64::MAX` on overflow
    /// GNU lib/coreutils include similar functionality
    /// and GNU test suite checks this behavior for some utils (`split` for example)
    pub fn parse_u64_max(&self, size: &str) -> Result<u64, ParseSizeError> {
        let result = self.parse_u64(size);
        match result {
            Ok(_) => result,
            Err(error) => {
                if let ParseSizeError::SizeTooBig(_) = error {
                    Ok(u64::MAX)
                } else {
                    Err(error)
                }
            }
        }
    }

    /// Same as `parse_u64_max()`, except for u128, i.e. returns `u128::MAX` on overflow
    pub fn parse_u128_max(&self, size: &str) -> Result<u128, ParseSizeError> {
        // let result = self.parse_u128(size);
        // match result {
        //     Ok(_) => result,
        //     Err(error) => {
        //         if let ParseSizeError::SizeTooBig(_) = error {
        //             Ok(u128::MAX)
        //         } else {
        //             Err(error)
        //         }
        //     }
        // }
        match self.parse_u128(size) {
            Ok(_) => self.parse_u128(size),
            Err(error) => match error {
                ParseSizeError::SizeTooBig(_) => Ok(u128::MAX),
                _ => Err(error),
            },
        }
    }

    fn determine_number_system(size: &str) -> NumberSystem {
        // if size.len() <= 1 {
        //     return NumberSystem::Decimal;
        // }
        //
        // if size.starts_with("0x") {
        //     return NumberSystem::Hexadecimal;
        // }
        //
        // let num_digits: usize = size
        //     .chars()
        //     .take_while(char::is_ascii_digit)
        //     .collect::<String>()
        //     .len();
        // let all_zeros = size.chars().all(|c| c == '0');
        // if size.starts_with('0') && num_digits > 1 && !all_zeros {
        //     return NumberSystem::Octal;
        // }
        //
        // NumberSystem::Decimal
        if size.len() <= 1 {
            return NumberSystem::Decimal;
        }

        if size.starts_with("0x") {
            return NumberSystem::Hexadecimal;
        }

        let mut iter = size.chars();
        if let Some('0') = iter.next() {
            if let Some(digit) = iter.next() {
                if digit.is_ascii_digit() && !iter.all(|c| c == '0') {
                    return NumberSystem::Octal;
                }
            }
        }

        NumberSystem::Decimal
    }

    fn parse_number(
        numeric_string: &str,
        radix: u32,
        original_size: &str,
    ) -> Result<u128, ParseSizeError> {
        // u128::from_str_radix(numeric_string, radix).map_err(|e| match e.kind() {
        //     IntErrorKind::PosOverflow => ParseSizeError::size_too_big(original_size),
        //     _ => ParseSizeError::ParseFailure(original_size.to_string()),
        // })
        // 调用原始函数并处理错误
        let result = u128::from_str_radix(numeric_string, radix).map_err(|e| match e.kind() {
            IntErrorKind::PosOverflow => ParseSizeError::size_too_big(original_size),
            _ => ParseSizeError::ParseFailure(original_size.to_string()),
        });

        match result {
            Ok(value) => Ok(value),
            Err(e) => Err(e),
        }
    }
}

/// Parse a size string into a number of bytes
/// using Default Parser (no custom settings)
///
/// # Examples
///
/// ```rust
/// use ctcore::ct_parse_size::parse_size_u128;
/// assert_eq!(parse_size_u128("123"),Ok(123),);
/// assert_eq!(parse_size_u128("9kB"),Ok(9 * 1000),); // kB is 1000
/// assert_eq!(parse_size_u128("2K"),Ok(2 * 1024),); // K is 1024
/// assert_eq!(parse_size_u128("0xACDBK"),Ok(44251 * 1024),);
/// ```
pub fn parse_size_u128(size: &str) -> Result<u128, ParseSizeError> {
    Parser::default().parse(size)
}

/// Same as `parse_size_u128()`, but for u64
pub fn parse_size_u64(size: &str) -> Result<u64, ParseSizeError> {
    Parser::default().parse_u64(size)
}

#[deprecated = "Please use parse_size_u64(size: &str) -> Result<u64, ParseSizeError> OR parse_size_u128(size: &str) -> Result<u128, ParseSizeError> instead."]
pub fn parse_size(size: &str) -> Result<u64, ParseSizeError> {
    parse_size_u64(size)
}

/// Same as `parse_size_u64()`, except returns `u64::MAX` on overflow
/// GNU lib/coreutils include similar functionality
/// and GNU test suite checks this behavior for some utils
pub fn parse_size_u64_max(size: &str) -> Result<u64, ParseSizeError> {
    Parser::default().parse_u64_max(size)
}

/// Same as `parse_size_u128()`, except returns `u128::MAX` on overflow
pub fn parse_size_u128_max(size: &str) -> Result<u128, ParseSizeError> {
    Parser::default().parse_u128_max(size)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseSizeError {
    InvalidSuffix(String), // Suffix
    ParseFailure(String),  // Syntax
    SizeTooBig(String),    // Overflow
}

impl Error for ParseSizeError {
    fn description(&self) -> &str {
        // match *self {
        //     Self::InvalidSuffix(ref s) => s,
        //     Self::ParseFailure(ref s) => s,
        //     Self::SizeTooBig(ref s) => s,
        // }

        match *self {
            ParseSizeError::InvalidSuffix(ref s) => s,
            ParseSizeError::ParseFailure(ref s) => s,
            ParseSizeError::SizeTooBig(ref s) => s,
        }
    }
}

impl fmt::Display for ParseSizeError {
    // fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    //     let s = match self {
    //         Self::InvalidSuffix(s) | Self::ParseFailure(s) | Self::SizeTooBig(s) => s,
    //     };
    //     write!(f, "{s}")
    // }

    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let (_error_type, message) = match self {
            Self::InvalidSuffix(s) => ("Invalid Suffix", s),
            Self::ParseFailure(s) => ("Parse Failure", s),
            Self::SizeTooBig(s) => ("Size Too Big", s),
        };
        write!(f, "{}", message)
    }
}

// FIXME: It's more idiomatic to move the formatting into the Display impl,
// but there's a lot of downstream code that constructs these errors manually
// that would be affected
impl ParseSizeError {
    fn invalid_suffix(s: &str) -> Self {
        Self::InvalidSuffix(format!("{}", s.quote()))
    }

    fn parse_failure(s: &str) -> Self {
        // stderr on linux (GNU coreutils 8.32) (LC_ALL=C)
        // has to be handled in the respective cttils because strings differ, e.g.:
        //
        // `NUM`
        // head:     invalid number of bytes: '1fb'
        // tail:     invalid number of bytes: '1fb'
        //
        // `SIZE`
        // split:    invalid number of bytes: '1fb'
        // truncate: Invalid number: '1fb'
        //
        // `MODE`
        // stdbuf:   invalid mode '1fb'
        //
        // `SIZE`
        // sort:     invalid suffix in --buffer-size argument '1fb'
        // sort:     invalid --buffer-size argument 'fb'
        //
        // `SIZE`
        // du:       invalid suffix in --buffer-size argument '1fb'
        // du:       invalid suffix in --threshold argument '1fb'
        // du:       invalid --buffer-size argument 'fb'
        // du:       invalid --threshold argument 'fb'
        //
        // `BYTES`
        // od:       invalid suffix in --read-bytes argument '1fb'
        // od:       invalid --read-bytes argument  argument 'fb'
        //                   --skip-bytes
        //                   --width
        //                   --strings
        // etc.
        Self::ParseFailure(format!("{}", s.quote()))
    }

    fn size_too_big(s: &str) -> Self {
        // stderr on linux (GNU coreutils 8.32) (LC_ALL=C)
        // has to be handled in the respective cttils because strings differ, e.g.:
        //
        // head:     invalid number of bytes: '1Y': Value too large for defined data type
        // tail:     invalid number of bytes: '1Y': Value too large for defined data type
        // split:    invalid number of bytes: '1Y': Value too large for defined data type
        // truncate:          Invalid number: '1Y': Value too large for defined data type
        // stdbuf:               invalid mode '1Y': Value too large for defined data type
        // sort:     -S argument '1Y' too large
        // du:       -B argument '1Y' too large
        // od:       -N argument '1Y' too large
        // etc.
        //
        // stderr on macos (brew - GNU coreutils 8.32) also differs for the same version, e.g.:
        // ghead:   invalid number of bytes: '1Y': Value too large to be stored in data type
        // gtail:   invalid number of bytes: '1Y': Value too large to be stored in data type
        Self::SizeTooBig(format!(
            "{}: Value too large for defined data type",
            s.quote()
        ))
    }
}

