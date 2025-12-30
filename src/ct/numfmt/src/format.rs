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
use ctcore::ct_display::Quotable;

use crate::flags::{NumfmtConfigs, NumfmtRoundMethod, NumfmtTransformOptions};
use crate::units::DisplayableSuffix;
use crate::units::NumfmtRawSuffix;
use crate::units::NumfmtSuffix;
use crate::units::NumfmtUnit;
use crate::units::Result;
use crate::units::NUMFMT_IEC_BASES;
use crate::units::NUMFMT_SI_BASES;

/// 遍历一行的字段，其中每个字段都是一个连续的非空格序列。
/// 非空格的连续序列，可选择以一个或多个前导空格字符作为前缀。
/// 白字符。字段以 `(prefix, field)`的元组形式返回。
///
/// # Examples:
///
/// ```
/// let mut fields = ct_numfmt::format::NumfmtWhitespaceSplitter { s: Some("    1234 5") };
///
/// assert_eq!(Some(("    ", "1234")), fields.next());
/// assert_eq!(Some((" ", "5")), fields.next());
/// assert_eq!(None, fields.next());
/// ```
///
/// 结果中包含分隔符；`prefix`仅在行的第一个字段为空（包括输入行为空的情况）：
///
/// ```
/// let mut fields = ct_numfmt::format::NumfmtWhitespaceSplitter { s: Some("first second") };
///
/// assert_eq!(Some(("", "first")), fields.next());
/// assert_eq!(Some((" ", "second")), fields.next());
///
/// let mut fields = ct_numfmt::format::NumfmtWhitespaceSplitter { s: Some("") };
///
/// assert_eq!(Some(("", "")), fields.next());
/// ```
pub struct NumfmtWhitespaceSplitter<'a> {
    pub s: Option<&'a str>,
}

impl<'a> Iterator for NumfmtWhitespaceSplitter<'a> {
    type Item = (&'a str, &'a str);

    /// 生成输入字符串中的下一个字段，作为一个元组 `(prefix, field)`。
    fn next(&mut self) -> Option<Self::Item> {
        let haystack = self.s?;

        let (prefix, field_tmp) = haystack.split_at(
            haystack
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(haystack.len()),
        );

        let (field, rest) = field_tmp.split_at(
            field_tmp
                .find(char::is_whitespace)
                .unwrap_or(field_tmp.len()),
        );

        self.s = match rest.is_empty() {
            true => None,
            false => Some(rest),
        };

        Some((prefix, field))
    }
}

fn numfmt_parse_suffix(s: &str) -> Result<(f64, Option<NumfmtSuffix>)> {
    if s.is_empty() {
        return Err("invalid number: ''".to_string());
    }

    let is_with_i = s.ends_with('i');
    let mut iter = s.chars();
    if is_with_i {
        iter.next_back();
    }
    let suffix = match iter.next_back() {
        Some('K') => Some((NumfmtRawSuffix::K, is_with_i)),
        Some('M') => Some((NumfmtRawSuffix::M, is_with_i)),
        Some('G') => Some((NumfmtRawSuffix::G, is_with_i)),
        Some('T') => Some((NumfmtRawSuffix::T, is_with_i)),
        Some('P') => Some((NumfmtRawSuffix::P, is_with_i)),
        Some('E') => Some((NumfmtRawSuffix::E, is_with_i)),
        Some('Z') => Some((NumfmtRawSuffix::Z, is_with_i)),
        Some('Y') => Some((NumfmtRawSuffix::Y, is_with_i)),
        Some('0'..='9') if !is_with_i => None,
        _ => return Err(format!("invalid suffix in input: {}", s.quote())),
    };

    let suffix_size = match suffix {
        None => 0,
        Some((_, false)) => 1,
        Some((_, true)) => 2,
    };

    let number = s[..s.len() - suffix_size]
        .parse::<f64>()
        .map_err(|_| format!("invalid number: {}", s.quote()))?;

    Ok((number, suffix))
}

// 返回数字的隐式精度，即点后面的位数。例如
// 例如，1.23 的隐含精度为 2。
fn numfmt_parse_implicit_precision(s: &str) -> usize {
    if let Some((_, decimal_part_value)) = s.split_once('.') {
        decimal_part_value
            .chars()
            .take_while(char::is_ascii_digit)
            .count()
    } else {
        0
    }
}

fn numfmt_remove_suffix(i: f64, s: Option<NumfmtSuffix>, u: &NumfmtUnit) -> Result<f64> {
    match (s, u) {
        (Some((raw_suffix, false)), &NumfmtUnit::Auto)
        | (Some((raw_suffix, false)), &NumfmtUnit::Si) => match raw_suffix {
            NumfmtRawSuffix::K => Ok(i * 1e3),
            NumfmtRawSuffix::M => Ok(i * 1e6),
            NumfmtRawSuffix::G => Ok(i * 1e9),
            NumfmtRawSuffix::T => Ok(i * 1e12),
            NumfmtRawSuffix::P => Ok(i * 1e15),
            NumfmtRawSuffix::E => Ok(i * 1e18),
            NumfmtRawSuffix::Z => Ok(i * 1e21),
            NumfmtRawSuffix::Y => Ok(i * 1e24),
        },
        (Some((raw_suffix, false)), &NumfmtUnit::Iec(false))
        | (Some((raw_suffix, true)), &NumfmtUnit::Auto)
        | (Some((raw_suffix, true)), &NumfmtUnit::Iec(true)) => match raw_suffix {
            NumfmtRawSuffix::K => Ok(i * NUMFMT_IEC_BASES[1]),
            NumfmtRawSuffix::M => Ok(i * NUMFMT_IEC_BASES[2]),
            NumfmtRawSuffix::G => Ok(i * NUMFMT_IEC_BASES[3]),
            NumfmtRawSuffix::T => Ok(i * NUMFMT_IEC_BASES[4]),
            NumfmtRawSuffix::P => Ok(i * NUMFMT_IEC_BASES[5]),
            NumfmtRawSuffix::E => Ok(i * NUMFMT_IEC_BASES[6]),
            NumfmtRawSuffix::Z => Ok(i * NUMFMT_IEC_BASES[7]),
            NumfmtRawSuffix::Y => Ok(i * NUMFMT_IEC_BASES[8]),
        },
        (None, &NumfmtUnit::Iec(true)) => {
            Err(format!("missing 'i' suffix in input: '{i}' (e.g Ki/Mi/Gi)"))
        }
        (Some((raw_suffix, false)), &NumfmtUnit::Iec(true)) => Err(format!(
            "missing 'i' suffix in input: '{i}{raw_suffix:?}' (e.g Ki/Mi/Gi)"
        )),
        (Some((raw_suffix, with_i)), &NumfmtUnit::None) => Err(format!(
            "rejecting suffix in input: '{}{:?}{}' (consider using --from)",
            i,
            raw_suffix,
            if with_i { "i" } else { "" }
        )),
        (None, _) => Ok(i),
        (_, _) => Err("This suffix is unsupported for specified unit".to_owned()),
    }
}

fn numfmt_transform_from(s: &str, numfmt_opts: &NumfmtTransformOptions) -> Result<f64> {
    let (i, suffix) = numfmt_parse_suffix(s)?;
    let i = i * (numfmt_opts.from_unit as f64);

    numfmt_remove_suffix(i, suffix, &numfmt_opts.from).map(|number| {
        // 如果用户没有提供 --from 参数，GNU numfmt 不会对数值进行四舍五入。
        if numfmt_opts.from == NumfmtUnit::None {
            if number == -0.0 {
                0.0
            } else {
                number
            }
        } else if number < 0.0 {
            -number.abs().ceil()
        } else {
            number.ceil()
        }
    })
}

/// 用分子除以分母，四舍五入。
///
/// 如果除法的结果小于 10.0，则四舍五入到小数点后一位。
///
/// 否则，四舍五入为整数。
///
/// # 例子:
///
/// ```
/// use ct_numfmt::format::numfmt_div_round;
/// use ct_numfmt::flags::NumfmtRoundMethod;
///
/// // 四舍五入方法：
/// assert_eq!(numfmt_div_round(1.01, 1.0, NumfmtRoundMethod::FromZero), 1.1);
/// assert_eq!(numfmt_div_round(1.01, 1.0, NumfmtRoundMethod::TowardsZero), 1.0);
/// assert_eq!(numfmt_div_round(1.01, 1.0, NumfmtRoundMethod::Up), 1.1);
/// assert_eq!(numfmt_div_round(1.01, 1.0, NumfmtRoundMethod::Down), 1.0);
/// assert_eq!(numfmt_div_round(1.01, 1.0, NumfmtRoundMethod::Nearest), 1.0);
///
/// // Division:
/// assert_eq!(numfmt_div_round(999.1, 1000.0, NumfmtRoundMethod::FromZero), 1.0);
/// assert_eq!(numfmt_div_round(1001., 10., NumfmtRoundMethod::FromZero), 101.);
/// assert_eq!(numfmt_div_round(9991., 10., NumfmtRoundMethod::FromZero), 1000.);
/// assert_eq!(numfmt_div_round(-12.34, 1.0, NumfmtRoundMethod::FromZero), -13.0);
/// assert_eq!(numfmt_div_round(1000.0, -3.14, NumfmtRoundMethod::FromZero), -319.0);
/// assert_eq!(numfmt_div_round(-271828.0, -271.0, NumfmtRoundMethod::FromZero), 1004.0);
/// ```
pub fn numfmt_div_round(n: f64, d: f64, round_method: NumfmtRoundMethod) -> f64 {
    let v = n / d;

    if v.abs() < 10.0 {
        round_method.round(10.0 * v) / 10.0
    } else {
        round_method.round(v)
    }
}

// 四舍五入到指定的小数点位数。
fn numfmt_round_with_precision(n: f64, round_method: NumfmtRoundMethod, precision: usize) -> f64 {
    let p = 10.0_f64.powf(precision as f64);

    round_method.round(p * n) / p
}

fn numfmt_consider_suffix(
    n: f64,
    u: &NumfmtUnit,
    round_method: NumfmtRoundMethod,
    precision: usize,
) -> Result<(f64, Option<NumfmtSuffix>)> {
    use crate::units::NumfmtRawSuffix::*;

    let n_abs = n.abs();
    let raw_suffixes = [K, M, G, T, P, E, Z, Y];

    let (bases, is_with_i) = match *u {
        NumfmtUnit::Si => (&NUMFMT_SI_BASES, false),
        NumfmtUnit::Iec(with_i) => (&NUMFMT_IEC_BASES, with_i),
        NumfmtUnit::Auto => return Err("Unit 'auto' isn't supported with --to options".to_owned()),
        NumfmtUnit::None => return Ok((n, None)),
    };

    let i = match n_abs {
        _ if n_abs <= bases[1] - 1.0 => return Ok((n, None)),
        _ if n_abs < bases[2] => 1,
        _ if n_abs < bases[3] => 2,
        _ if n_abs < bases[4] => 3,
        _ if n_abs < bases[5] => 4,
        _ if n_abs < bases[6] => 5,
        _ if n_abs < bases[7] => 6,
        _ if n_abs < bases[8] => 7,
        _ if n_abs < bases[9] => 8,
        _ => return Err("Number is too big and unsupported".to_string()),
    };

    let v = if precision > 0 {
        numfmt_round_with_precision(n / bases[i], round_method, precision)
    } else {
        numfmt_div_round(n, bases[i], round_method)
    };

    // 检查四舍五入是否将我们推入下一个基数
    if v.abs() >= bases[1] {
        Ok((v / bases[1], Some((raw_suffixes[i], is_with_i))))
    } else {
        Ok((v, Some((raw_suffixes[i - 1], is_with_i))))
    }
}

fn numfmt_transform_to(
    s: f64,
    opts: &NumfmtTransformOptions,
    round_method: NumfmtRoundMethod,
    precision: usize,
) -> Result<String> {
    let (i2, suffix) = numfmt_consider_suffix(s, &opts.to, round_method, precision)?;
    let i2 = i2 / (opts.to_unit as f64);
    Ok(match suffix {
        None => {
            format!(
                "{:.precision$}",
                numfmt_round_with_precision(i2, round_method, precision),
                precision = precision
            )
        }
        Some(numfmt_suffix) if precision > 0 => {
            format!(
                "{:.precision$}{}",
                i2,
                DisplayableSuffix(numfmt_suffix),
                precision = precision
            )
        }
        Some(numfmt_suffix) if i2.abs() < 10.0 => {
            format!("{:.1}{}", i2, DisplayableSuffix(numfmt_suffix))
        }
        Some(numfmt_suffix) => format!("{:.0}{}", i2, DisplayableSuffix(numfmt_suffix)),
    })
}

fn numfmt_format_string(
    source: &str,
    numfmt_configs: &NumfmtConfigs,
    implicit_padding: Option<isize>,
) -> Result<String> {
    // 在应用任何转换之前，先去除（optional）后缀
    let source_without_suffix = match &numfmt_configs.suffix {
        Some(suffix) => source.strip_suffix(suffix).unwrap_or(source),
        None => source,
    };

    let precision = if let Some(p) = numfmt_configs.format.precision {
        p
    } else if numfmt_configs.transform.from == NumfmtUnit::None
        && numfmt_configs.transform.to == NumfmtUnit::None
    {
        numfmt_parse_implicit_precision(source_without_suffix)
    } else {
        0
    };

    let number = numfmt_transform_to(
        numfmt_transform_from(source_without_suffix, &numfmt_configs.transform)?,
        &numfmt_configs.transform,
        numfmt_configs.round,
        precision,
    )?;

    // 在应用填充之前，恢复后缀
    let number_with_suffix = match &numfmt_configs.suffix {
        Some(suffix) => format!("{number}{suffix}"),
        None => number,
    };

    let padded_number = get_padded_number(numfmt_configs, implicit_padding, number_with_suffix);

    Ok(format!(
        "{}{}{}",
        numfmt_configs.format.prefix, padded_number, numfmt_configs.format.suffix
    ))
}

fn get_padded_number(
    numfmt_configs: &NumfmtConfigs,
    implicit_padding: Option<isize>,
    number_with_suffix: String,
) -> String {
    let padding = numfmt_configs
        .format
        .padding
        .unwrap_or_else(|| implicit_padding.unwrap_or(numfmt_configs.padding));

    match padding {
        0 => number_with_suffix,
        p_isize if p_isize > 0 && numfmt_configs.format.is_zero_padding => {
            let zero_padded = format!(
                "{:0>padding$}",
                number_with_suffix,
                padding = p_isize as usize
            );

            match implicit_padding.unwrap_or(numfmt_configs.padding) {
                0 => zero_padded,
                p if p > 0 => format!("{:>padding$}", zero_padded, padding = p as usize),
                p => format!("{:<padding$}", zero_padded, padding = p.unsigned_abs()),
            }
        }
        p_isize if p_isize > 0 => format!(
            "{:>padding$}",
            number_with_suffix,
            padding = p_isize as usize
        ),
        p_isize => format!(
            "{:<padding$}",
            number_with_suffix,
            padding = p_isize.unsigned_abs()
        ),
    }
}

fn numfmt_format_and_print_delimited(s: &str, options: &NumfmtConfigs) -> Result<()> {
    let delimiter = options.delimiter.as_ref().unwrap();

    for (n, field) in (1..).zip(s.split(delimiter)) {
        let is_field_selected = ctcore::ct_ranges::contain(&options.fields, n);

        // 在第二个及其后的字段前打印分隔符
        if n > 1 {
            print!("{}", delimiter);
        }

        if is_field_selected {
            print!(
                "{}",
                numfmt_format_string(field.trim_start(), options, None)?
            );
        } else {
            // 打印未选择的字段，不进行转换
            print!("{}", field);
        }
    }

    println!();

    Ok(())
}

fn numfmt_format_and_print_whitespace(s: &str, numfmt_configs: &NumfmtConfigs) -> Result<()> {
    for (n, (prefix, field)) in (1..).zip(NumfmtWhitespaceSplitter { s: Some(s) }) {
        let is_field_selected = ctcore::ct_ranges::contain(&numfmt_configs.fields, n);

        if is_field_selected {
            let is_empty_prefix = prefix.is_empty();

            // 在第二个及其后的字段前打印分隔符
            let prefix_str = if n > 1 {
                print!(" ");
                &prefix[1..]
            } else {
                prefix
            };

            let implicit_padding = if !is_empty_prefix && numfmt_configs.padding == 0 {
                Some((prefix_str.len() + field.len()) as isize)
            } else {
                None
            };

            print!(
                "{}",
                numfmt_format_string(field, numfmt_configs, implicit_padding)?
            );
        } else {
            // 打印未选择的字段，不进行转换
            print!("{}{}", prefix, field);
        }
    }

    println!();

    Ok(())
}

/// 根据所选选项格式化一行文本。
///
/// 给定一行文本 "s"，将该行文本拆分成若干字段，对选定的数字字段进行转换和ct_format，
/// 并将结果打印到stdout。未被选中转换的字段将原封不动地通过。
pub fn numfmt_format_and_print(s: &str, numfmt_configs: &NumfmtConfigs) -> Result<()> {
    if numfmt_configs.delimiter.is_some() {
        numfmt_format_and_print_delimited(s, numfmt_configs)
    } else {
        numfmt_format_and_print_whitespace(s, numfmt_configs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::NumfmtInvalidModes;

    fn setup_default_config() -> NumfmtConfigs {
        NumfmtConfigs {
            transform: NumfmtTransformOptions {
                from: NumfmtUnit::Auto,
                from_unit: 1,
                to: NumfmtUnit::Si,
                to_unit: 1,
            },
            padding: 0,
            header: 0,
            fields: vec![],
            delimiter: None,
            round: NumfmtRoundMethod::Up,
            suffix: None,
            format: Default::default(),
            invalid: NumfmtInvalidModes::Abort,
        }
    }

    #[cfg(test)]
    mod numfmt_parse_suffix_tests {
        use crate::format::numfmt_parse_suffix;
        use crate::units::NumfmtRawSuffix;

        #[test]
        fn test_parse_with_suffix() {
            let examples = [
                ("100K", 100.0, Some((NumfmtRawSuffix::K, false))),
                ("200Mi", 200.0, Some((NumfmtRawSuffix::M, true))),
                ("300G", 300.0, Some((NumfmtRawSuffix::G, false))),
                ("400Ti", 400.0, Some((NumfmtRawSuffix::T, true))),
                ("500P", 500.0, Some((NumfmtRawSuffix::P, false))),
                ("600Ei", 600.0, Some((NumfmtRawSuffix::E, true))),
                ("700Z", 700.0, Some((NumfmtRawSuffix::Z, false))),
                ("800Yi", 800.0, Some((NumfmtRawSuffix::Y, true))),
            ];
            for (input, expected_value, expected_suffix) in examples {
                let result = numfmt_parse_suffix(input).unwrap();
                assert_eq!(result, (expected_value, expected_suffix));
            }
        }

        #[test]
        fn test_parse_without_suffix() {
            let result = numfmt_parse_suffix("1234").unwrap();
            assert_eq!(result, (1234.0, None));
        }

        // #[test]
        // fn test_parse_invalid_suffix() {
        //     let inputs = ["100Kx", "200 Mi", "300Gi", "400.25T"];
        //     for input in inputs {
        //         assert!(numfmt_parse_suffix(input).is_err(), "Expected error for input: {}", input);
        //     }
        // }

        #[test]
        fn test_parse_empty_string() {
            assert!(
                numfmt_parse_suffix("").is_err(),
                "Expected error for empty input"
            );
        }

        #[test]
        fn test_parse_extreme_values() {
            let large_number = format!("{}K", f64::MAX);
            let small_number = format!("{}M", f64::MIN);
            assert!(
                numfmt_parse_suffix(&large_number).is_ok(),
                "Should handle large values"
            );
            assert!(
                numfmt_parse_suffix(&small_number).is_ok(),
                "Should handle small values"
            );
        }

        #[test]
        fn test_parse_with_non_standard_characters_in_suffix() {
            assert!(
                numfmt_parse_suffix("100K3").is_err(),
                "Expected error for non-standard characters in suffix"
            );
        }

        #[test]
        fn test_parse_with_multiple_suffixes() {
            assert!(
                numfmt_parse_suffix("100KM").is_err(),
                "Expected error for multiple suffixes"
            );
        }

        #[test]
        fn test_parse_with_spaces() {
            assert!(
                numfmt_parse_suffix("100 K").is_err(),
                "Expected error for space between number and suffix"
            );
            assert!(
                numfmt_parse_suffix("100 M ").is_err(),
                "Expected error for space after suffix"
            );
        }

        #[test]
        fn test_parse_with_additional_characters_between_number_and_suffix() {
            assert!(
                numfmt_parse_suffix("100xK").is_err(),
                "Expected error for additional characters between number and suffix"
            );
        }

        #[test]
        fn test_parse_with_high_precision_decimals() {
            let result = numfmt_parse_suffix("123.456789K").unwrap();
            assert_eq!(
                result,
                (123.456789, Some((NumfmtRawSuffix::K, false))),
                "Should parse high precision decimals correctly"
            );
        }

        #[test]
        fn test_parse_with_non_ascii_characters() {
            assert!(
                numfmt_parse_suffix("100€K").is_err(),
                "Expected error for non-ASCII characters"
            );
            assert!(
                numfmt_parse_suffix("100K€").is_err(),
                "Expected error for non-ASCII characters after suffix"
            );
        }
    }
    #[cfg(test)]
    mod parse_implicit_precision_tests {
        use super::*;
        #[test]
        fn test_no_decimal_point() {
            assert_eq!(numfmt_parse_implicit_precision("100"), 0);
        }

        #[test]
        fn test_decimal_point_no_digits() {
            assert_eq!(numfmt_parse_implicit_precision("100."), 0);
        }

        #[test]
        fn test_normal_decimal_point_with_digits() {
            assert_eq!(numfmt_parse_implicit_precision("100.1234"), 4);
        }

        #[test]
        fn test_decimal_point_with_non_digit_characters() {
            assert_eq!(numfmt_parse_implicit_precision("100.1234abc"), 4);
        }

        #[test]
        fn test_multiple_decimal_points() {
            // 实际上这不是一个合法的浮点数表示，但测试可以用来看看函数怎样处理这种输入
            assert_eq!(numfmt_parse_implicit_precision("100.12.34"), 2);
        }
    }
    #[cfg(test)]
    mod remove_suffix_tests {
        use super::*;
        // 假设NUMFMT_IEC_BASES数组如下
        const NUMFMT_IEC_BASES: [f64; 9] = [
            1.0,
            1024.0,
            1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
            1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        ];

        #[test]
        fn test_si_suffix_handling() {
            let i = 1.0;
            let suffixes = vec![
                (NumfmtRawSuffix::K, 1e3),
                (NumfmtRawSuffix::M, 1e6),
                (NumfmtRawSuffix::G, 1e9),
                (NumfmtRawSuffix::T, 1e12),
            ];
            for (suffix, multiplier) in suffixes {
                let result =
                    numfmt_remove_suffix(i, Some((suffix, false)), &NumfmtUnit::Si).unwrap();
                assert_eq!(result, i * multiplier);
            }
        }

        #[test]
        fn test_iec_suffix_handling() {
            let i = 1.0;
            let unit = NumfmtUnit::Iec(true);
            for (index, suffix) in [
                NumfmtRawSuffix::K,
                NumfmtRawSuffix::M,
                NumfmtRawSuffix::G,
                NumfmtRawSuffix::T,
            ]
            .iter()
            .enumerate()
            {
                let result = numfmt_remove_suffix(i, Some((*suffix, true)), &unit).unwrap();
                assert_eq!(result, i * NUMFMT_IEC_BASES[index + 1]);
            }
        }

        #[test]
        fn test_error_handling_mismatch_suffix_unit() {
            let result =
                numfmt_remove_suffix(1.0, Some((NumfmtRawSuffix::K, true)), &NumfmtUnit::Si);
            assert!(
                result.is_err(),
                "Expected an error due to mismatch suffix and unit"
            );
        }

        #[test]
        fn test_no_suffix_handling() {
            let i = 123.0;
            let result = numfmt_remove_suffix(i, None, &NumfmtUnit::Si).unwrap();
            assert_eq!(
                result, i,
                "Expected the original number when no suffix is provided"
            );
        }

        #[test]
        fn test_unsupported_suffix_error() {
            let result =
                numfmt_remove_suffix(1.0, Some((NumfmtRawSuffix::Z, false)), &NumfmtUnit::None);
            assert!(
                result.is_err(),
                "Expected an error for unsupported suffix with NumfmtUnit::None"
            );
        }
        #[test]
        fn test_large_value_suffix_handling() {
            let i = 1e12; // 极大数值
            let result =
                numfmt_remove_suffix(i, Some((NumfmtRawSuffix::T, false)), &NumfmtUnit::Si)
                    .unwrap();
            assert_eq!(result, i * 1e12);
        }

        #[test]
        fn test_small_value_suffix_handling() {
            let i = 1e-12; // 极小数值
            let result =
                numfmt_remove_suffix(i, Some((NumfmtRawSuffix::K, false)), &NumfmtUnit::Si)
                    .unwrap();
            assert_eq!(result, i * 1e3);
        }

        #[test]
        fn test_special_float_values_handling() {
            // let nan_result = numfmt_remove_suffix(f64::NAN, Some((NumfmtRawSuffix::K, false)), &NumfmtUnit::Si);
            // assert_eq!(nan_result.unwrap(), f64::NAN);

            let inf_result = numfmt_remove_suffix(
                f64::INFINITY,
                Some((NumfmtRawSuffix::M, false)),
                &NumfmtUnit::Si,
            );
            println!("{:?}", inf_result);
            assert_eq!(inf_result.unwrap(), f64::INFINITY);
        }

        #[test]
        fn test_complete_mismatch_error_handling() {
            let result = numfmt_remove_suffix(
                1.0,
                Some((NumfmtRawSuffix::P, false)),
                &NumfmtUnit::Iec(true),
            );
            assert!(
                result.is_err(),
                "Expected an error due to complete mismatch of suffix and unit settings"
            );
        }

        #[test]
        fn test_detailed_iec_units() {
            let i = 1.0;
            let unit = NumfmtUnit::Iec(true);
            let suffixes = [
                (NumfmtRawSuffix::K, NUMFMT_IEC_BASES[1]),
                (NumfmtRawSuffix::M, NUMFMT_IEC_BASES[2]),
                (NumfmtRawSuffix::G, NUMFMT_IEC_BASES[3]),
            ];
            for (suffix, base) in suffixes.iter() {
                let result = numfmt_remove_suffix(i, Some((*suffix, true)), &unit).unwrap();
                assert_eq!(result, i * base);
            }
        }
    }
}