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
use ctcore::ct_parse_size::{ParseSizeError, parse_size_u64};

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
    // 解析基数和起始位置
    let (start, radix) = parse_radix_and_start(s);

    // 如果不是特殊格式，使用标准解析
    if radix == 10 && start == 0 {
        return parse_size_u64(&s[start..]);
    }

    // 解析数值和单位
    let (len, multiply) = parse_unit_suffix(s, radix)?;

    // 解析数值并计算最终结果
    calculate_final_value(&s[start..len], radix, multiply)
}

/// 解析数字的基数和起始位置
fn parse_radix_and_start(s: &str) -> (usize, u32) {
    if s.starts_with("0x") || s.starts_with("0X") {
        (2, 16)
    } else if s.starts_with('0') {
        (0, 8)
    } else {
        (0, 10)
    }
}

/// 解析单位后缀并返回相应的乘数
fn parse_unit_suffix(s: &str, radix: u32) -> Result<(usize, u64), ParseSizeError> {
    let mut chars = s.chars().rev();
    let mut len = s.len();
    let mut multiply = 1;

    match chars.next() {
        Some('b') if radix != 16 => {
            len -= 1;
            multiply = 512;
            Ok((len, multiply))
        }
        Some('k' | 'K') => {
            len -= 1;
            multiply = 1024;
            Ok((len, multiply))
        }
        Some('m' | 'M') => {
            len -= 1;
            multiply = 1024 * 1024;
            Ok((len, multiply))
        }
        Some('G') => {
            len -= 1;
            multiply = 1024 * 1024 * 1024;
            Ok((len, multiply))
        }
        #[cfg(target_pointer_width = "64")]
        Some('T') => {
            len -= 1;
            multiply = 1024 * 1024 * 1024 * 1024;
            Ok((len, multiply))
        }
        #[cfg(target_pointer_width = "64")]
        Some('P') => {
            len -= 1;
            multiply = 1024 * 1024 * 1024 * 1024 * 1024;
            Ok((len, multiply))
        }
        #[cfg(target_pointer_width = "64")]
        Some('E') if radix != 16 => {
            len -= 1;
            multiply = 1024 * 1024 * 1024 * 1024 * 1024 * 1024;
            Ok((len, multiply))
        }
        Some('B') if radix != 16 => parse_binary_prefix(s, &mut chars, len),
        _ => Ok((len, multiply)),
    }
}

/// 解析二进制前缀（kB、MB等）
fn parse_binary_prefix(
    s: &str,
    chars: &mut std::iter::Rev<std::str::Chars>,
    mut len: usize,
) -> Result<(usize, u64), ParseSizeError> {
    len -= 2;
    let multiply = match chars.next() {
        Some('k' | 'K') => 1000,
        Some('m' | 'M') => 1000 * 1000,
        Some('G') => 1000 * 1000 * 1000,
        #[cfg(target_pointer_width = "64")]
        Some('T') => 1000 * 1000 * 1000 * 1000,
        #[cfg(target_pointer_width = "64")]
        Some('P') => 1000 * 1000 * 1000 * 1000 * 1000,
        #[cfg(target_pointer_width = "64")]
        Some('E') => 1000 * 1000 * 1000 * 1000 * 1000 * 1000,
        _ => return Err(ParseSizeError::ParseFailure(s.to_string())),
    };
    Ok((len, multiply))
}

/// 计算最终的字节数值
fn calculate_final_value(s: &str, radix: u32, multiply: u64) -> Result<u64, ParseSizeError> {
    let factor =
        u64::from_str_radix(s, radix).map_err(|e| ParseSizeError::ParseFailure(e.to_string()))?;

    factor
        .checked_mul(multiply)
        .ok_or_else(|| ParseSizeError::SizeTooBig(s.to_string()))
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
    fn test_parse_errors() {
        // 测试错误情况
        assert!(od_parse_number_of_bytes("").is_err());
        assert!(od_parse_number_of_bytes("abc").is_err());
        assert!(od_parse_number_of_bytes("1xB").is_err());
        assert!(od_parse_number_of_bytes("0x1g").is_err());
    }
}
