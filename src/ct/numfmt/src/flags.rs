/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */
use std::str::FromStr;

use crate::units::NumfmtUnit;
use ctcore::ct_ranges::CtRange;
#[derive(Debug, PartialEq)]
pub struct NumfmtTransformOptions {
    pub from: NumfmtUnit,
    pub from_unit: usize,
    pub to: NumfmtUnit,
    pub to_unit: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NumfmtInvalidModes {
    Abort,
    Fail,
    Warn,
    Ignore,
}

#[derive(Debug, PartialEq)]
pub struct NumfmtConfigs {
    pub transform: NumfmtTransformOptions,
    pub padding: isize,
    pub header: usize,
    pub fields: Vec<CtRange>,
    pub delimiter: Option<String>,
    pub round: NumfmtRoundMethod,
    pub suffix: Option<String>,
    pub format: NumfmtFormatOptions,
    pub invalid: NumfmtInvalidModes,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumfmtRoundMethod {
    Up,
    Down,
    FromZero,
    TowardsZero,
    Nearest,
}

impl NumfmtRoundMethod {
    pub fn round(&self, f_value: f64) -> f64 {
        match self {
            NumfmtRoundMethod::Up => f_value.ceil(),
            NumfmtRoundMethod::Down => f_value.floor(),
            NumfmtRoundMethod::FromZero => {
                if f_value < 0.0 {
                    f_value.floor()
                } else {
                    f_value.ceil()
                }
            }
            NumfmtRoundMethod::TowardsZero => {
                if f_value < 0.0 {
                    f_value.ceil()
                } else {
                    f_value.floor()
                }
            }
            NumfmtRoundMethod::Nearest => f_value.round(),
        }
    }
}

// 代表从用户提供的 --format 参数中提取的选项。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct NumfmtFormatOptions {
    pub is_grouping: bool,
    pub padding: Option<isize>,
    pub precision: Option<usize>,
    pub prefix: String,
    pub suffix: String,
    pub is_zero_padding: bool,
}

impl FromStr for NumfmtFormatOptions {
    type Err = String;

    // 识别的 format 为: [PREFIX]%[0]['][-][N][.][N]f[SUFFIX]
    //
    // format定义了浮点参数"%f "的打印。
    // 可选的引号（%'f）可以实现--分组。
    // 可选的宽度值（%10f）将填充数字。
    // 可选的零值（%010f）将使数字归零。
    // 可选的负值（%-10f）将左对齐。
    // 可选精度值（%.1f）决定数字的精度。
    #[allow(clippy::cognitive_complexity)]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars_iter = s.chars().peekable();
        let mut numfmt_options = Self::default();

        let mut padding = String::new();
        let mut precision = String::new();
        let mut double_percentage_counter = 0;

        // 前缀中的"%"字符（如果有）必须以偶数长度出现，例如 "%%%%" 和 "%% %%"可以，"%%% %"不行。单个"%"字符将被视为浮点参数的开头。
        while let Some(c) = chars_iter.next() {
            match c {
                '%' if chars_iter.peek() == Some(&'%') => {
                    chars_iter.next();
                    double_percentage_counter += 1;

                    for _ in 0..2 {
                        numfmt_options.prefix.push('%');
                    }
                }
                '%' => break,
                _ => numfmt_options.prefix.push(c),
            }
        }

        // GNU numfmt 会在前缀中每出现一个"%%"就从前缀中删除一个字符，因此我们也要这样做
        for _ in 0..double_percentage_counter {
            numfmt_options.prefix.pop();
        }

        if chars_iter.peek().is_none() {
            return if numfmt_options.prefix == s {
                Err(format!("format '{s}' has no % directive"))
            } else {
                Err(format!("format '{s}' ends in %"))
            };
        }

        // GNU numfmt 允许以任何方式混合字符" "、"'"和 "0"，因此我们也这样做
        while matches!(chars_iter.peek(), Some(' ' | '\'' | '0')) {
            match chars_iter.next().unwrap() {
                ' ' => (),
                '\'' => numfmt_options.is_grouping = true,
                '0' => numfmt_options.is_zero_padding = true,
                _ => unreachable!(),
            }
        }

        if let Some('-') = chars_iter.peek() {
            chars_iter.next();

            match chars_iter.peek() {
                Some(c) if c.is_ascii_digit() => padding.push('-'),
                _ => {
                    return Err(format!(
                        "invalid format '{s}', directive must be %[0]['][-][N][.][N]f"
                    ))
                }
            }
        }

        while let Some(c) = chars_iter.peek() {
            if c.is_ascii_digit() {
                padding.push(*c);
                chars_iter.next();
            } else {
                break;
            }
        }

        if !padding.is_empty() {
            if let Ok(p) = padding.parse() {
                numfmt_options.padding = Some(p);
            } else {
                return Err(format!("invalid format '{s}' (width overflow)"));
            }
        }

        if let Some('.') = chars_iter.peek() {
            chars_iter.next();

            if matches!(chars_iter.peek(), Some(' ' | '+' | '-')) {
                return Err(format!("invalid precision in format '{s}'"));
            }

            while let Some(c) = chars_iter.peek() {
                if c.is_ascii_digit() {
                    precision.push(*c);
                    chars_iter.next();
                } else {
                    break;
                }
            }

            if precision.is_empty() {
                numfmt_options.precision = Some(0);
            } else if let Ok(p) = precision.parse() {
                numfmt_options.precision = Some(p);
            } else {
                return Err(format!("invalid precision in format '{s}'"));
            }
        }

        if let Some('f') = chars_iter.peek() {
            chars_iter.next();
        } else {
            return Err(format!(
                "invalid format '{s}', directive must be %[0]['][-][N][.][N]f"
            ));
        }

        // 后缀中如果有"%"字符，必须以偶数长度出现，否则就是错误。例如 "%%%%"和"%% %%"可以，"%%% %"不行。
        while let Some(c) = chars_iter.next() {
            if c != '%' {
                numfmt_options.suffix.push(c);
            } else if chars_iter.peek() == Some(&'%') {
                for _ in 0..2 {
                    numfmt_options.suffix.push('%');
                }
                chars_iter.next();
            } else {
                return Err(format!("format '{s}' has too many % directives"));
            }
        }

        Ok(numfmt_options)
    }
}

impl FromStr for NumfmtInvalidModes {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "abort" => Ok(NumfmtInvalidModes::Abort),
            "fail" => Ok(NumfmtInvalidModes::Fail),
            "warn" => Ok(NumfmtInvalidModes::Warn),
            "ignore" => Ok(NumfmtInvalidModes::Ignore),
            unknown => Err(format!("Unknown invalid mode: {unknown}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_format() {
        assert_eq!(NumfmtFormatOptions::default(), "%f".parse().unwrap());
        assert_eq!(NumfmtFormatOptions::default(), "%  f".parse().unwrap());
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_parse_format_with_invalid_formats() {
        assert!("".parse::<NumfmtFormatOptions>().is_err());
        assert!("hello".parse::<NumfmtFormatOptions>().is_err());
        assert!("hello%".parse::<NumfmtFormatOptions>().is_err());
        assert!("%-f".parse::<NumfmtFormatOptions>().is_err());
        assert!("%d".parse::<NumfmtFormatOptions>().is_err());
        assert!("%4 f".parse::<NumfmtFormatOptions>().is_err());
        assert!("%f%".parse::<NumfmtFormatOptions>().is_err());
        assert!("%f%%%".parse::<NumfmtFormatOptions>().is_err());
        assert!("%%f".parse::<NumfmtFormatOptions>().is_err());
        assert!("%%%%f".parse::<NumfmtFormatOptions>().is_err());
        assert!("%.-1f".parse::<NumfmtFormatOptions>().is_err());
        assert!("%. 1f".parse::<NumfmtFormatOptions>().is_err());
        assert!("%18446744073709551616f"
            .parse::<NumfmtFormatOptions>()
            .is_err());
        assert!("%.18446744073709551616f"
            .parse::<NumfmtFormatOptions>()
            .is_err());
    }

    #[test]
    fn test_parse_format_with_prefix_and_suffix() {
        let formats = vec![
            ("--%f", "--", ""),
            ("%f::", "", "::"),
            ("--%f::", "--", "::"),
            ("%f%%", "", "%%"),
            ("%%%f", "%", ""),
            ("%% %f", "%%", ""),
        ];

        for (format, expected_prefix, expected_suffix) in formats {
            let options: NumfmtFormatOptions = format.parse().unwrap();
            assert_eq!(expected_prefix, options.prefix);
            assert_eq!(expected_suffix, options.suffix);
        }
    }

    #[test]
    fn test_parse_format_with_padding() {
        let mut expected = NumfmtFormatOptions::default();
        let formats = vec![("%12f", Some(12)), ("%-12f", Some(-12))];

        for (format, expected_padding) in formats {
            expected.padding = expected_padding;
            assert_eq!(expected, format.parse().unwrap());
        }
    }

    #[test]
    fn test_parse_format_with_precision() {
        let mut expected = NumfmtFormatOptions::default();
        let formats = vec![
            ("%6.2f", Some(6), Some(2)),
            ("%6.f", Some(6), Some(0)),
            ("%.2f", None, Some(2)),
            ("%.f", None, Some(0)),
        ];

        for (format, expected_padding, expected_precision) in formats {
            expected.padding = expected_padding;
            expected.precision = expected_precision;
            assert_eq!(expected, format.parse().unwrap());
        }
    }

    #[test]
    fn test_parse_format_with_grouping() {
        let expected = NumfmtFormatOptions {
            is_grouping: true,
            ..Default::default()
        };
        assert_eq!(expected, "%'f".parse().unwrap());
        assert_eq!(expected, "% ' f".parse().unwrap());
        assert_eq!(expected, "%'''''''f".parse().unwrap());
    }

    #[test]
    fn test_parse_format_with_zero_padding() {
        let expected = NumfmtFormatOptions {
            padding: Some(10),
            is_zero_padding: true,
            ..Default::default()
        };
        assert_eq!(expected, "%010f".parse().unwrap());
        assert_eq!(expected, "% 0 10f".parse().unwrap());
        assert_eq!(expected, "%0000000010f".parse().unwrap());
    }

    #[test]
    fn test_parse_format_with_grouping_and_zero_padding() {
        let expected = NumfmtFormatOptions {
            is_grouping: true,
            is_zero_padding: true,
            ..Default::default()
        };
        assert_eq!(expected, "%0'f".parse().unwrap());
        assert_eq!(expected, "%'0f".parse().unwrap());
        assert_eq!(expected, "%0'0'0'f".parse().unwrap());
        assert_eq!(expected, "%'0'0'0f".parse().unwrap());
    }

    #[test]
    fn test_set_invalid_mode() {
        assert_eq!(
            Ok(NumfmtInvalidModes::Abort),
            NumfmtInvalidModes::from_str("abort")
        );
        assert_eq!(
            Ok(NumfmtInvalidModes::Abort),
            NumfmtInvalidModes::from_str("ABORT")
        );

        assert_eq!(
            Ok(NumfmtInvalidModes::Fail),
            NumfmtInvalidModes::from_str("fail")
        );
        assert_eq!(
            Ok(NumfmtInvalidModes::Fail),
            NumfmtInvalidModes::from_str("FAIL")
        );

        assert_eq!(
            Ok(NumfmtInvalidModes::Ignore),
            NumfmtInvalidModes::from_str("ignore")
        );
        assert_eq!(
            Ok(NumfmtInvalidModes::Ignore),
            NumfmtInvalidModes::from_str("IGNORE")
        );

        assert_eq!(
            Ok(NumfmtInvalidModes::Warn),
            NumfmtInvalidModes::from_str("warn")
        );
        assert_eq!(
            Ok(NumfmtInvalidModes::Warn),
            NumfmtInvalidModes::from_str("WARN")
        );

        assert!(NumfmtInvalidModes::from_str("something unknown").is_err());
    }
}