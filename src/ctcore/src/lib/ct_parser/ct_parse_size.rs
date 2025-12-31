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

/// 用于解析以 SI 或 IEC 单位（1000 或 1024 字节倍数）表示的尺寸的解析器。
///
/// 使用 [CtParser::parse] 函数执行解析。
///
#[derive(Default)]
pub struct CtParser<'parser> {
    /// 是否允许空数字字符串。
    pub no_empty_numeric: bool,
    /// 是否将后缀 "B" 视为表示 "bytes"。
    pub capital_b_bytes: bool,
    /// 是否将 "b" 视为 "byte count" 而非 "block"
    pub b_byte_count: bool,
    /// 后缀白名单
    pub allow_list: Option<&'parser [&'parser str]>,
    /// 未提供后缀时使用的默认单位
    pub default_unit: Option<&'parser str>,
}

#[derive(PartialEq)]
enum CtNumberSystem {
    Decimal,
    Octal,
    Hexadecimal,
}

impl<'parser> CtParser<'parser> {
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
    /// 将大小字符串解析为字节数。
    ///
    /// 大小字符串由一个整数和一个可选单位组成。单位可以是K、M、G、T、P、E、Z、Y、R或Q（1024的幂），也可以是KB、MB等（1000的幂），或者是b表示512。
    /// 也可以使用二进制前缀：KiB=K，MiB=M，以此类推。
    ///
    /// # 错误
    ///
    /// 如果无法将此字符串解析为数字，则返回ParseSizeError，例如，字符串不是以数字开始，或者单位不是上一节所描述的支持单位之一。
    ///
    /// # 示例
    ///
    /// ```rust
    /// use ctcore::ct_parse_size::CtParser;
    /// let parser = CtParser {
    ///     default_unit: Some("M"),
    ///     ..Default::default()
    /// };
    /// assert_eq!(Ok(123 * 1024 * 1024), parser.parse("123M")); // M is 1024^2
    /// assert_eq!(Ok(123 * 1024 * 1024), parser.parse("123")); // default unit set to "M" on ct_parser instance
    /// assert_eq!(Ok(9 * 1000), parser.parse("9kB")); // kB is 1000
    /// assert_eq!(Ok(2 * 1024), parser.parse("2K")); // K is 1024
    /// assert_eq!(Ok(44251 * 1024), parser.parse("0xACDBK")); // 0xACDB is 44251 in decimal
    /// ```
    pub fn parse(&self, size: &str) -> Result<u128, ParseSizeError> {
        if size.is_empty() {
            return Err(ParseSizeError::parse_failure(size));
        }

        let number_system = Self::determine_number_system(size);

        // 将尺寸参数拆分为数字部分和单位部分
        // 例如，若参数为 "123K"，则数字部分为 "123"，单位为 "K"
        let numeric_string: String = match number_system {
            CtNumberSystem::Hexadecimal => size
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

        // 检查b是否为字节计数并移除b
        if self.b_byte_count && unit.ends_with('b') {
            // 若 unit = 'b' 则返回错误
            if numeric_string.is_empty() {
                return Err(ParseSizeError::parse_failure(size));
            }
            unit = &unit[0..unit.len() - 1];
        }

        if let Some(allow_list) = self.allow_list {
            // 检查unit是否出现在allow_list中，如果不在则返回错误
            if !allow_list.contains(&unit) && !unit.is_empty() {
                if numeric_string.is_empty() {
                    return Err(ParseSizeError::parse_failure(size));
                }
                return Err(ParseSizeError::invalid_suffix(size));
            }
        }

        // 计算单位所代表的因子。
        // 空字符串表示因子为 1。
        //
        // 小写字母 "b" （被 od、head、tail 等工具使用）表示“块”，而 Posix 块大小为 512。大写字母 "B" 则表示“字节”。
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

        // 将字符串解析为u128
        let number: u128;
        if number_system == CtNumberSystem::Decimal {
            if numeric_string.is_empty() && !self.no_empty_numeric {
                number = 1;
            } else {
                number = Self::parse_number(&numeric_string, 10, size)?;
            }
        } else if number_system == CtNumberSystem::Octal {
            let trimmed_string = numeric_string.trim_start_matches('0');
            number = Self::parse_number(trimmed_string, 8, size)?;
        } else if number_system == CtNumberSystem::Hexadecimal {
            let trimmed_string = numeric_string.trim_start_matches("0x");
            number = Self::parse_number(trimmed_string, 16, size)?;
        } else {
            // 如有必要处理其他情况
            unreachable!("Unexpected number system encountered");
        }

        match number.checked_mul(factor) {
            Some(result) => Ok(result),
            None => Err(ParseSizeError::size_too_big(size)),
        }
    }

    /// parse()的显式u128别名
    pub fn parse_u128(&self, size: &str) -> Result<u128, ParseSizeError> {
        self.parse(size)
    }

    /// 与 parse() 相同，但尝试返回 u64 类型结果
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

    /// 与parse_u64()相同，只是在溢出时返回u64::MAX。
    /// GNU lib/coreutils包含类似功能，
    /// 并且GNU测试套件针对某些实用程序（例如split）检查这种行为
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

    /// 与 parse_u64_max() 类似，但针对 u128 类型，即在溢出时返回 u128::MAX
    pub fn parse_u128_max(&self, size: &str) -> Result<u128, ParseSizeError> {
        match self.parse_u128(size) {
            Ok(_) => self.parse_u128(size),
            Err(error) => match error {
                ParseSizeError::SizeTooBig(_) => Ok(u128::MAX),
                _ => Err(error),
            },
        }
    }

    fn determine_number_system(size: &str) -> CtNumberSystem {
        if size.len() <= 1 {
            return CtNumberSystem::Decimal;
        }

        if size.starts_with("0x") {
            return CtNumberSystem::Hexadecimal;
        }

        let mut iter = size.chars();
        if let Some('0') = iter.next() {
            if let Some(digit) = iter.next() {
                if digit.is_ascii_digit() && !iter.all(|c| c == '0') {
                    return CtNumberSystem::Octal;
                }
            }
        }

        CtNumberSystem::Decimal
    }

    fn parse_number(
        numeric_string: &str,
        radix: u32,
        original_size: &str,
    ) -> Result<u128, ParseSizeError> {
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
    CtParser::default().parse(size)
}

/// Same as `parse_size_u128()`, but for u64
pub fn parse_size_u64(size: &str) -> Result<u64, ParseSizeError> {
    CtParser::default().parse_u64(size)
}

#[deprecated = "Please use parse_size_u64(size: &str) -> Result<u64, ParseSizeError> OR parse_size_u128(size: &str) -> Result<u128, ParseSizeError> instead."]
pub fn parse_size(size: &str) -> Result<u64, ParseSizeError> {
    parse_size_u64(size)
}

/// Same as `parse_size_u64()`, except returns `u64::MAX` on overflow
/// GNU lib/coreutils include similar functionality
/// and GNU test suite checks this behavior for some utils
pub fn parse_size_u64_max(size: &str) -> Result<u64, ParseSizeError> {
    CtParser::default().parse_u64_max(size)
}

/// Same as `parse_size_u128()`, except returns `u128::MAX` on overflow
pub fn parse_size_u128_max(size: &str) -> Result<u128, ParseSizeError> {
    CtParser::default().parse_u128_max(size)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseSizeError {
    InvalidSuffix(String), // Suffix
    ParseFailure(String),  // Syntax
    SizeTooBig(String),    // Overflow
}

impl Error for ParseSizeError {
    fn description(&self) -> &str {
        match *self {
            ParseSizeError::InvalidSuffix(ref s) => s,
            ParseSizeError::ParseFailure(ref s) => s,
            ParseSizeError::SizeTooBig(ref s) => s,
        }
    }
}

impl fmt::Display for ParseSizeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let (_error_type, message) = match self {
            Self::InvalidSuffix(s) => ("Invalid Suffix", s),
            Self::ParseFailure(s) => ("Parse Failure", s),
            Self::SizeTooBig(s) => ("Size Too Big", s),
        };
        write!(f, "{}", message)
    }
}

impl ParseSizeError {
    fn invalid_suffix(s: &str) -> Self {
        Self::InvalidSuffix(format!("{}", s.quote()))
    }

    fn parse_failure(s: &str) -> Self {
        // 因为字符串不同，所以必须在相应的cttils中处理，例如：
        //
        // `NUM`
        // head:     无效的字节数量：'1fb'
        // tail:     无效的字节数量：'1fb'
        //
        // `SIZE`
        // split:    无效的字节数量：'1fb'
        // truncate: 无效数字：'1fb'
        //
        // `MODE`
        // stdbuf:   无效模式 '1fb'
        //
        // `SIZE`
        // sort:     --buffer-size 参数中存在无效后缀 '1fb'
        // sort:     无效的 --buffer-size 参数 'fb'
        //
        // `SIZE`
        // du:       --buffer-size 参数中存在无效后缀 '1fb'
        // du:       --threshold 参数中存在无效后缀 '1fb'
        // du:       无效的 --buffer-size 参数 'fb'
        // du:       无效的 --threshold 参数 'fb'
        //
        // `BYTES`
        // od:       --read-bytes 参数中存在无效后缀 '1fb'
        // od:       无效的 --read-bytes 参数 'fb'
        //                   --skip-bytes
        //                   --width
        //                   --strings
        Self::ParseFailure(format!("{}", s.quote()))
    }

    fn size_too_big(s: &str) -> Self {
        // 因为字符串不同，所以需要在各自的 ctutils 中进行处理，例如：
        //
        // 示例错误输出：
        // head:     非法的字节数量：'1Y'，值过大，超过已定义的数据类型范围
        // tail:     非法的字节数量：'1Y'，值过大，超过已定义的数据类型范围
        // split:    非法的字节数量：'1Y'，值过大，超过已定义的数据类型范围
        // truncate:          非法的数值：'1Y'，值过大，超过已定义的数据类型范围
        // stdbuf:               非法的模式 '1Y'，值过大，超过已定义的数据类型范围
        // sort:     -S参数 '1Y' 值过大
        // du:       -B参数 '1Y' 值过大
        // od:       -N参数 '1Y' 值过大
        // 等等。
        // 同样版本的GNU coreutils在macOS（通过Homebrew安装）上的标准错误输出也存在差异，例如：
        // ghead:   非法的字节数量：'1Y'，值过大，无法存储在数据类型中
        // gtail:   非法的字节数量：'1Y'，值过大，无法存储在数据类型中

        Self::SizeTooBig(format!(
            "{}: Value too large for defined data type",
            s.quote()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ct_parse_size::{CtParser, ParseSizeError, parse_size_u64, parse_size_u128};


     #[test]
    fn all_suffixes() {
        let suffixes = [
            ('K', 1u32),
            ('M', 2u32),
            ('G', 3u32),
            ('T', 4u32),
            ('P', 5u32),
            ('E', 6u32),
            ('Z', 7u32),
            ('Y', 8u32),
            ('R', 9u32),
            ('Q', 10u32),
        ];

        for &(c, exp) in &suffixes {
            let s = format!("2{c}B"); // KB
            assert_eq!(Ok(2 * (1000_u128).pow(exp)), parse_size_u128(&s));
            let s = format!("2{c}"); // K
            assert_eq!(Ok(2 * (1024_u128).pow(exp)), parse_size_u128(&s));
            let s = format!("2{c}iB"); // KiB
            assert_eq!(Ok(2 * (1024_u128).pow(exp)), parse_size_u128(&s));
            let s = format!("2{}iB", c.to_lowercase()); // kiB
            assert_eq!(Ok(2 * (1024_u128).pow(exp)), parse_size_u128(&s));

            let s = format!("{c}B"); // KB
            assert_eq!(Ok((1000_u128).pow(exp)), parse_size_u128(&s));
            let s = format!("{c}"); // K
            assert_eq!(Ok((1024_u128).pow(exp)), parse_size_u128(&s));
            let s = format!("{c}iB"); // KiB
            assert_eq!(Ok((1024_u128).pow(exp)), parse_size_u128(&s));
            let s = format!("{}iB", c.to_lowercase()); // kiB
            assert_eq!(Ok((1024_u128).pow(exp)), parse_size_u128(&s));
        }
    }

    #[test]
    fn overflow_x64() {
        assert!(parse_size_u64("10000000000000000000000").is_err());
        assert!(parse_size_u64("1000000000T").is_err());
        assert!(parse_size_u64("100000P").is_err());
        assert!(parse_size_u64("100E").is_err());
        assert!(parse_size_u64("1Z").is_err());
        assert!(parse_size_u64("1Y").is_err());
        assert!(parse_size_u64("1R").is_err());
        assert!(parse_size_u64("1Q").is_err());

        assert_eq!(
            ParseSizeError::SizeTooBig(String::from("'1Z': Value too large for defined data type")),
            parse_size_u64("1Z").unwrap_err()
        );

        assert_eq!(
            ParseSizeError::SizeTooBig("'1Y': Value too large for defined data type".to_string()),
            parse_size_u64("1Y").unwrap_err()
        );
        assert_eq!(
            ParseSizeError::SizeTooBig("'1R': Value too large for defined data type".to_string()),
            parse_size_u64("1R").unwrap_err()
        );
        assert_eq!(
            ParseSizeError::SizeTooBig("'1Q': Value too large for defined data type".to_string()),
            parse_size_u64("1Q").unwrap_err()
        );
    }

    #[test]
    fn overflow_to_max_u64() {
        assert_eq!(Ok(1_099_511_627_776), parse_size_u64_max("1T"));
        assert_eq!(Ok(1_125_899_906_842_624), parse_size_u64_max("1P"));
        assert_eq!(Ok(u64::MAX), parse_size_u64_max("18446744073709551616"));
        assert_eq!(Ok(u64::MAX), parse_size_u64_max("10000000000000000000000"));
        assert_eq!(Ok(u64::MAX), parse_size_u64_max("1Y"));
        assert_eq!(Ok(u64::MAX), parse_size_u64_max("1R"));
        assert_eq!(Ok(u64::MAX), parse_size_u64_max("1Q"));
    }

    #[test]
    fn overflow_to_max_u128() {
        assert_eq!(
            Ok(12_379_400_392_853_802_748_991_242_240),
            parse_size_u128_max("10R")
        );
        assert_eq!(
            Ok(12_676_506_002_282_294_014_967_032_053_760),
            parse_size_u128_max("10Q")
        );
        assert_eq!(Ok(u128::MAX), parse_size_u128_max("1000000000000R"));
        assert_eq!(Ok(u128::MAX), parse_size_u128_max("1000000000Q"));
    }

    #[test]
    fn invalid_suffix() {
        let test_strings = ["5mib", "1eb", "1H"];
        let result_strings = ["'5mib'", "'1eb'", "'1H'"];
        for (i, test_string) in test_strings.iter().enumerate() {
            assert_eq!(
                parse_size_u64(&test_string).unwrap_err(),
                ParseSizeError::InvalidSuffix(result_strings[i].to_string())
            );
        }
    }

    #[test]
    fn parse_size_invalid_str() {
        let test_strings = ["x", "", "abc"];
        let result_strings = ["'x'", "''", "'abc'"];
        for (i, test_string) in test_strings.iter().enumerate() {
            assert_eq!(
                parse_size_u64(&test_string).unwrap_err(),
                ParseSizeError::ParseFailure(result_strings[i].to_string())
            );
        }
    }

    #[test]
    fn zero() {
        assert_eq!(Ok(0), parse_size_u64("0"));
        assert_eq!(Ok(0), parse_size_u128("0"));
    }

    fn variant_eq(a: &ParseSizeError, b: &ParseSizeError) -> bool {
        std::mem::discriminant(b) == std::mem::discriminant(a)
    }

    #[test]
    fn test_base_all_suffixes() {
        // 单位可以是K、M、G、T、P、E、Z、Y、R或Q（1024的幂），也可以是KB、MB等（1000的幂）。
        // 也可以使用二进制前缀：KiB=K，MiB=M，以此类推。
        let suffixes = [
            ('K', 1u32),
            ('M', 2u32),
            ('G', 3u32),
            ('T', 4u32),
            ('P', 5u32),
            ('E', 6u32),
            ('Z', 7u32),
            ('Y', 8u32),
            ('R', 9u32),
            ('Q', 10u32),
        ];

        for &(c, exp) in &suffixes {
            let s = format!("2{c}B"); // KB
            assert_eq!(parse_size_u128(&s), Ok(2 * (1000_u128).pow(exp)));
            let s = format!("2{c}"); // K
            assert_eq!(parse_size_u128(&s), Ok(2 * (1024_u128).pow(exp)));
            let s = format!("2{c}iB"); // KiB
            assert_eq!(parse_size_u128(&s), Ok(2 * (1024_u128).pow(exp)));
            let s = format!("2{}iB", c.to_lowercase()); // kiB
            assert_eq!(parse_size_u128(&s), Ok(2 * (1024_u128).pow(exp)));

            // suffix only
            let s = format!("{c}B"); // KB
            assert_eq!(parse_size_u128(&s), Ok((1000_u128).pow(exp)));
            let s = format!("{c}"); // K
            assert_eq!(parse_size_u128(&s), Ok((1024_u128).pow(exp)));
            let s = format!("{c}iB"); // KiB
            assert_eq!(parse_size_u128(&s), Ok((1024_u128).pow(exp)),);
            let s = format!("{}iB", c.to_lowercase()); // kiB
            assert_eq!(parse_size_u128(&s), Ok((1024_u128).pow(exp)));
        }
    }

    #[test]
    fn test_base_overflow_x64() {
        assert!(parse_size_u64("10000000000000000000000").is_err());
        assert!(parse_size_u64("1000000000T").is_err());
        assert!(parse_size_u64("100000P").is_err());
        assert!(parse_size_u64("100E").is_err());
        assert!(parse_size_u64("1Z").is_err());
        assert!(parse_size_u64("1Y").is_err());
        assert!(parse_size_u64("1R").is_err());
        assert!(parse_size_u64("1Q").is_err());

        assert!(variant_eq(
            &ParseSizeError::SizeTooBig(String::new()),
            &parse_size_u64("1Z").unwrap_err()
        ));

        assert_eq!(
            parse_size_u64("1Y").unwrap_err(),
            ParseSizeError::SizeTooBig("'1Y': Value too large for defined data type".to_string()),
        );
        assert_eq!(
            parse_size_u64("1R").unwrap_err(),
            ParseSizeError::SizeTooBig("'1R': Value too large for defined data type".to_string()),
        );
        assert_eq!(
            parse_size_u64("1Q").unwrap_err(),
            ParseSizeError::SizeTooBig("'1Q': Value too large for defined data type".to_string()),
        );
    }

    #[test]
    fn test_base_overflow_to_max_u64() {
        assert_eq!(parse_size_u64_max("1Y"), Ok(u64::MAX));

        assert_eq!(parse_size_u64_max("1Q"), Ok(u64::MAX));
        assert_eq!(parse_size_u64_max("1R"), Ok(u64::MAX));
        assert_eq!(parse_size_u64_max("1T"), Ok(1_099_511_627_776));
        assert_eq!(parse_size_u64_max("1P"), Ok(1_125_899_906_842_624));

        assert_eq!(parse_size_u64_max("10000000000000000000000"), Ok(u64::MAX));
        assert_eq!(parse_size_u64_max("18446744073709551616"), Ok(u64::MAX));
    }

    #[test]
    fn test_base_overflow_to_max_u128() {
        assert_eq!(
            parse_size_u128_max("10R"),
            Ok(12_379_400_392_853_802_748_991_242_240),
        );
        assert_eq!(
            parse_size_u128_max("10Q"),
            Ok(12_676_506_002_282_294_014_967_032_053_760),
        );
        assert_eq!(parse_size_u128_max("1000000000000R"), Ok(u128::MAX),);
        assert_eq!(parse_size_u128_max("1000000000Q"), Ok(u128::MAX),);
    }

    #[test]
    fn test_base_invalid_suffix() {
        let test_strings = ["5mib", "1eb", "1H"];
        for &test_string in &test_strings {
            assert_eq!(
                ParseSizeError::InvalidSuffix(format!("{}", test_string.quote())),
                parse_size_u64(test_string).unwrap_err(),
            );
        }
    }

    #[test]
    fn test_base_invalid_syntax() {
        let test_strings = ["biB", "-", "+", "", "-1", "∞"];
        for &test_string in &test_strings {
            assert_eq!(
                ParseSizeError::ParseFailure(format!("{}", test_string.quote())),
                parse_size_u64(test_string).unwrap_err(),
            );
        }
    }

    #[test]
    fn test_base_b_suffix() {
        assert_eq!(parse_size_u64("3b"), Ok(3 * 512),); // b is 512
    }

    #[test]
    fn test_base_no_suffix() {
        assert_eq!(parse_size_u64("1234"), Ok(1234),);
        assert_eq!(parse_size_u64("0"), Ok(0),);
        assert_eq!(parse_size_u64("5"), Ok(5),);
        assert_eq!(parse_size_u64("999"), Ok(999),);
    }

    #[test]
    fn test_base_kilobytes_suffix() {
        assert_eq!(parse_size_u64("123KB"), Ok(123 * 1000),); // KB is 1000
        assert_eq!(parse_size_u64("9kB"), Ok(9 * 1000),); // kB is 1000
        assert_eq!(parse_size_u64("2K"), Ok(2 * 1024),); // K is 1024
        assert_eq!(parse_size_u64("0K"), Ok(0),);
        assert_eq!(parse_size_u64("0KB"), Ok(0),);
        assert_eq!(parse_size_u64("KB"), Ok(1000),);
        assert_eq!(parse_size_u64("K"), Ok(1024),);
        assert_eq!(parse_size_u64("2kB"), Ok(2000),);
        assert_eq!(parse_size_u64("4KB"), Ok(4000),);
    }

    #[test]
    fn test_base_megabytes_suffix() {
        assert_eq!(parse_size_u64("123M"), Ok(123 * 1024 * 1024));
        assert_eq!(parse_size_u64("123MB"), Ok(123 * 1000 * 1000));
        assert_eq!(parse_size_u64("M"), Ok(1024 * 1024));
        assert_eq!(parse_size_u64("MB"), Ok(1000 * 1000));
        assert_eq!(parse_size_u64("2m"), Ok(2 * 1_048_576));
        assert_eq!(parse_size_u64("4M"), Ok(4 * 1_048_576));
        assert_eq!(parse_size_u64("2mB"), Ok(2_000_000));
        assert_eq!(parse_size_u64("4MB"), Ok(4_000_000));
    }

    #[test]
    fn test_base_gigabytes_suffix() {
        assert_eq!(parse_size_u64("1G"), Ok(1_073_741_824));
        assert_eq!(parse_size_u64("2GB"), Ok(2_000_000_000));
    }

  #[test]
    #[cfg(target_pointer_width = "64")]
    fn xtest_base_64() {
        assert_eq!(parse_size_u64("1T"), Ok(1_099_511_627_776));
        assert_eq!(parse_size_u64("1P"), Ok(1_125_899_906_842_624));
        assert_eq!(parse_size_u64("1E"), Ok(1_152_921_504_606_846_976));

        assert_eq!(parse_size_u128("1Z"), Ok(1_180_591_620_717_411_303_424));
        assert_eq!(parse_size_u128("1Y"), Ok(1_208_925_819_614_629_174_706_176));
        assert_eq!(
            parse_size_u128("1R"),
            Ok(1_237_940_039_285_380_274_899_124_224)
        );
        assert_eq!(
            parse_size_u128("1Q"),
            Ok(1_267_650_600_228_229_401_496_703_205_376)
        );

        assert_eq!(parse_size_u64("2TB"), Ok(2_000_000_000_000),);
        assert_eq!(parse_size_u64("2PB"), Ok(2_000_000_000_000_000),);
        assert_eq!(parse_size_u64("2EB"), Ok(2_000_000_000_000_000_000),);

        assert_eq!(parse_size_u128("2ZB"), Ok(2_000_000_000_000_000_000_000),);
        assert_eq!(
            parse_size_u128("2YB"),
            Ok(2_000_000_000_000_000_000_000_000)
        );
        assert_eq!(
            parse_size_u128("2RB"),
            Ok(2_000_000_000_000_000_000_000_000_000)
        );
        assert_eq!(
            parse_size_u128("2QB"),
            Ok(2_000_000_000_000_000_000_000_000_000_000)
        );
    }

    #[test]
    fn test_base_parse_size_options() {
        let mut parser = CtParser::default();

        parser
            .with_allow_list(&["k", "K", "G", "MB", "M"])
            .with_default_unit("K");

        assert_eq!(parser.parse("1"), Ok(1024));
        assert_eq!(parser.parse("2"), Ok(2 * 1024));
        assert_eq!(parser.parse("1MB"), Ok(1000 * 1000));
        assert_eq!(parser.parse("1M"), Ok(1024 * 1024));
        assert_eq!(parser.parse("1G"), Ok(1024 * 1024 * 1024));

        assert!(parser.parse("1P").is_err());
        assert!(parser.parse("1T").is_err());

        assert!(parser.parse("1E").is_err());

        parser
            .with_allow_list(&[
                "b", "k", "K", "m", "M", "MB", "g", "G", "t", "T", "P", "E", "Z", "Y", "R", "Q",
            ])
            .with_default_unit("K")
            .with_b_byte_count(true);

        assert_eq!(parser.parse("1"), Ok(1024));
        assert_eq!(parser.parse("2"), Ok(2 * 1024));
        assert_eq!(parser.parse("1MB"), Ok(1000 * 1000));
        assert_eq!(parser.parse("1M"), Ok(1024 * 1024));
        assert_eq!(parser.parse("1G"), Ok(1024 * 1024 * 1024));
        assert_eq!(
            parser.parse_u128("1R"),
            Ok(1_237_940_039_285_380_274_899_124_224)
        );
        assert_eq!(
            parser.parse_u128("1Q"),
            Ok(1_267_650_600_228_229_401_496_703_205_376)
        );

        assert_eq!(Ok(1), parser.parse("1b"));
        assert_eq!(Ok(1023), parser.parse("1023b"));
        assert_eq!(Ok(1023 * 1024 * 1024), parser.parse("1023Mb"));

        assert!(parser.parse("1B").is_err());
        assert!(parser.parse("b").is_err());
        assert!(parser.parse("B").is_err());
    }

    #[test]
    fn test_base_parse_octal_size() {
        assert_eq!(parse_size_u64("076"), Ok(62));
        assert_eq!(parse_size_u64("01017"), Ok(527));
        assert_eq!(parse_size_u128("01233K"), Ok(667 * 1024));
    }

    #[test]
    fn test_base_parse_hex_size() {
        assert_eq!(parse_size_u64("0xB"), Ok(11));
        assert_eq!(parse_size_u64("0x17203"), Ok(94723));
        assert_eq!(parse_size_u128("0xACDCK"), Ok(44252 * 1024));
    }

}