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
use ctcore::ct_parse_size::ParseSizeError;

/// 解析字节数字符串，支持多种格式和单位
///
/// # 参数
/// * `s` - 要解析的字符串
///
/// # 返回值
/// * `Result<u64, ParseSizeError>` - 解析成功返回字节数，失败返回错误
///
/// # 格式说明
/// * 支持十进制、八进制（0开头）和十六进制（0x开头）
/// * 支持以下单位后缀：
///   - b: 乘以512
///   - k/K: 乘以1024
///   - m/M: 乘以1024^2
///   - G: 乘以1024^3
///   - T: 乘以1024^4（仅64位系统）
///   - P: 乘以1024^5（仅64位系统）
///   - E: 乘以1024^6（仅64位系统）
///   - xB: x可以是k/m/G/T/P/E，表示乘以1000的相应次方
pub fn od_parse_number_of_bytes(s: &str) -> Result<u64, ParseSizeError> {
    let parsed = parse_number_part(s)?;
    let multiplier = parse_suffix(s, parsed.suffix_start)?;
    let value = parsed
        .number
        .checked_mul(multiplier)
        .ok_or_else(|| ParseSizeError::SizeTooBig(s.to_string()))?;

    u64::try_from(value).map_err(|_| ParseSizeError::SizeTooBig(s.to_string()))
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedNumber {
    number: u128,
    suffix_start: usize,
}

fn parse_number_part(s: &str) -> Result<ParsedNumber, ParseSizeError> {
    if s.is_empty() {
        return Err(ParseSizeError::ParseFailure(s.to_string()));
    }

    let bytes = s.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }

    if pos == bytes.len() {
        return Err(ParseSizeError::ParseFailure(s.to_string()));
    }

    let skipped_whitespace = pos > 0;
    if bytes[pos] == b'-' {
        return Err(ParseSizeError::ParseFailure(s.to_string()));
    }

    let had_plus = bytes[pos] == b'+';
    if had_plus {
        pos += 1;
        if pos == bytes.len() || !bytes[pos].is_ascii_digit() {
            return Err(ParseSizeError::ParseFailure(s.to_string()));
        }
    }

    if !bytes[pos].is_ascii_digit() {
        if !skipped_whitespace && !had_plus && is_valid_first_suffix(bytes[pos]) {
            return Ok(ParsedNumber {
                number: 1,
                suffix_start: pos,
            });
        }
        return Err(ParseSizeError::ParseFailure(s.to_string()));
    }

    let (radix, digits_start, mut suffix_start) =
        if bytes[pos] == b'0' && pos + 2 < bytes.len() && matches!(bytes[pos + 1], b'x' | b'X') {
            if bytes[pos + 2].is_ascii_hexdigit() {
                let digits_start = pos + 2;
                let mut end = digits_start;
                while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                    end += 1;
                }
                (16, digits_start, end)
            } else {
                (8, pos, pos + 1)
            }
        } else if bytes[pos] == b'0' {
            let digits_start = pos;
            let mut end = digits_start;
            while end < bytes.len() && matches!(bytes[end], b'0'..=b'7') {
                end += 1;
            }
            (8, digits_start, end)
        } else {
            let digits_start = pos;
            let mut end = digits_start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            (10, digits_start, end)
        };

    if digits_start == suffix_start {
        suffix_start = digits_start + 1;
    }

    let number = u128::from_str_radix(&s[digits_start..suffix_start], radix)
        .map_err(|_| ParseSizeError::SizeTooBig(s.to_string()))?;

    Ok(ParsedNumber {
        number,
        suffix_start,
    })
}

fn is_valid_first_suffix(c: u8) -> bool {
    matches!(
        c,
        b'b' | b'E' | b'G' | b'K' | b'k' | b'M' | b'm' | b'P' | b'Q' | b'R' | b'T' | b'Y' | b'Z'
    )
}

fn parse_suffix(s: &str, suffix_start: usize) -> Result<u128, ParseSizeError> {
    let suffix = &s[suffix_start..];
    let bytes = suffix.as_bytes();
    if bytes.is_empty() {
        return Ok(1);
    }

    if bytes[0] == b'b' {
        return if bytes.len() == 1 {
            Ok(512)
        } else {
            Err(ParseSizeError::InvalidSuffix(s.to_string()))
        };
    }

    let exponent = match bytes[0] {
        b'K' | b'k' => 1,
        b'M' | b'm' => 2,
        b'G' => 3,
        b'T' => 4,
        b'P' => 5,
        b'E' => 6,
        b'Z' => 7,
        b'Y' => 8,
        b'R' => 9,
        b'Q' => 10,
        _ => return Err(ParseSizeError::InvalidSuffix(s.to_string())),
    };

    let (base, suffix_len) = match bytes.get(1).copied() {
        None => (1024_u128, 1),
        Some(b'B' | b'D') => (1000_u128, 2),
        Some(b'i') if bytes.get(2) == Some(&b'B') => (1024_u128, 3),
        _ => return Err(ParseSizeError::InvalidSuffix(s.to_string())),
    };

    if bytes.len() != suffix_len {
        return Err(ParseSizeError::InvalidSuffix(s.to_string()));
    }

    Ok(base.pow(exponent))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_parse_number_of_bytes() {
        // 八进制输入测试
        assert_eq!(8, od_parse_number_of_bytes("010").unwrap());
        assert_eq!(8 * 512, od_parse_number_of_bytes("010b").unwrap());
        assert_eq!(8 * 1024, od_parse_number_of_bytes("010k").unwrap());
        assert_eq!(8 * 1_048_576, od_parse_number_of_bytes("010m").unwrap());

        // 十六进制输入测试
        assert_eq!(15, od_parse_number_of_bytes("0xf").unwrap());
        assert_eq!(14, od_parse_number_of_bytes("0XE").unwrap());
        assert_eq!(15, od_parse_number_of_bytes("0XF").unwrap());
        assert_eq!(27, od_parse_number_of_bytes("0x1b").unwrap());
        assert_eq!(16 * 1024, od_parse_number_of_bytes("0x10k").unwrap());
        assert_eq!(16 * 1_048_576, od_parse_number_of_bytes("0x10m").unwrap());
    }

    #[test]
    fn test_parse_binary_units() {
        // 测试二进制单位
        assert_eq!(1024, od_parse_number_of_bytes("1k").unwrap());
        assert_eq!(1024 * 1024, od_parse_number_of_bytes("1m").unwrap());
        assert_eq!(1024 * 1024 * 1024, od_parse_number_of_bytes("1G").unwrap());
    }

    #[test]
    fn test_parse_decimal_units() {
        // 测试十进制单位
        assert_eq!(1000, od_parse_number_of_bytes("1kB").unwrap());
        assert_eq!(1_000_000, od_parse_number_of_bytes("1MB").unwrap());
        assert_eq!(1_000_000_000, od_parse_number_of_bytes("1GB").unwrap());
    }

    #[test]
    fn test_parse_gnu_suffixes() {
        assert_eq!(0, od_parse_number_of_bytes("0ZiB").unwrap());
        assert_eq!(0, od_parse_number_of_bytes("00ZiB").unwrap());
        assert_eq!(0, od_parse_number_of_bytes("+0ZiB").unwrap());
        assert_eq!(0, od_parse_number_of_bytes("0x0ZiB").unwrap());
        assert_eq!(0, od_parse_number_of_bytes("0ZB").unwrap());
        assert_eq!(0, od_parse_number_of_bytes("0Y").unwrap());
        assert_eq!(0, od_parse_number_of_bytes("0R").unwrap());
        assert_eq!(0, od_parse_number_of_bytes("0Q").unwrap());
        assert_eq!(0, od_parse_number_of_bytes("0KiB").unwrap());
        assert_eq!(16 * 1000, od_parse_number_of_bytes("0x10KB").unwrap());
        assert_eq!(16 * 1024, od_parse_number_of_bytes("0x10KiB").unwrap());
        assert_eq!(1000, od_parse_number_of_bytes("KD").unwrap());
    }

    #[test]
    fn test_parse_errors() {
        // 测试错误情况
        assert!(od_parse_number_of_bytes("").is_err());
        assert!(od_parse_number_of_bytes("abc").is_err());
        assert!(od_parse_number_of_bytes("1xB").is_err());
        assert!(od_parse_number_of_bytes("0x1g").is_err());
        assert!(matches!(
            od_parse_number_of_bytes("0zB"),
            Err(ParseSizeError::InvalidSuffix(_))
        ));
        assert!(matches!(
            od_parse_number_of_bytes("0ziB"),
            Err(ParseSizeError::InvalidSuffix(_))
        ));
        assert!(matches!(
            od_parse_number_of_bytes("1Z"),
            Err(ParseSizeError::SizeTooBig(_))
        ));
    }
}
