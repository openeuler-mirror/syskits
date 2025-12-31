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
use crate::units::NUMFMT_IEC_BASES;
use crate::units::NUMFMT_SI_BASES;
use crate::units::NumfmtRawSuffix;
use crate::units::NumfmtSuffix;
use crate::units::NumfmtUnit;
use crate::units::Result;

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
            if number == -0.0 { 0.0 } else { number }
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
    #[cfg(test)]
    mod transform_from_tests {
        use super::*;

        #[test]
        fn test_basic_conversion() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::None,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(numfmt_transform_from("400", &opts).unwrap(), 400.0);
        }

        #[test]
        fn test_with_unit_and_rounding() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(numfmt_transform_from("100K", &opts).unwrap(), 100000.0);
        }

        #[test]
        fn test_negative_value_handling() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(numfmt_transform_from("-300K", &opts).unwrap(), -300000.0);
        }

        #[test]
        fn test_error_handling() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert!(numfmt_transform_from("unsupported", &opts).is_err());
        }

        #[test]
        fn test_large_unit_conversion() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(numfmt_transform_from("1M", &opts).unwrap(), 1_000_000.0);
        }

        #[test]
        fn test_iec_unit_conversion() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Iec(true),
                from_unit: 1024,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(
                numfmt_transform_from("1K", &opts).unwrap_err(),
                "missing 'i' suffix in input: '1024K' (e.g Ki/Mi/Gi)".to_string()
            );
        }

        #[test]
        fn test_very_small_values() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert_eq!(numfmt_transform_from("0.001K", &opts).unwrap(), 1.0);
        }

        #[test]
        fn test_special_float_values() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert!(numfmt_transform_from("NaN", &opts).is_err());
            assert!(numfmt_transform_from("inf", &opts).is_err());
        }

        #[test]
        fn test_invalid_multi_unit_input() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Si,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            };
            assert!(
                numfmt_transform_from("100KM", &opts).is_err(),
                "Expected an error for invalid multi-unit input"
            );
        }
    }

    #[cfg(test)]
    mod numfmt_div_round_tests {
        use super::*;
        #[test]
        fn test_basic_division_and_rounding() {
            assert_eq!(numfmt_div_round(10.0, 3.0, NumfmtRoundMethod::Up), 3.4);
            assert_eq!(numfmt_div_round(10.0, 3.0, NumfmtRoundMethod::Down), 3.3);
            assert_eq!(numfmt_div_round(10.0, 3.0, NumfmtRoundMethod::Nearest), 3.3);
        }

        #[test]
        fn test_decimal_precision() {
            assert_eq!(numfmt_div_round(1.0, 3.0, NumfmtRoundMethod::Up), 0.4);
            assert_eq!(numfmt_div_round(1.0, 3.0, NumfmtRoundMethod::Down), 0.3);
            assert_eq!(numfmt_div_round(1.0, 3.0, NumfmtRoundMethod::Nearest), 0.3);
        }

        #[test]
        fn test_small_values_rounding() {
            assert_eq!(numfmt_div_round(0.05, 2.0, NumfmtRoundMethod::Up), 0.1);
            assert_eq!(numfmt_div_round(0.05, 2.0, NumfmtRoundMethod::Down), 0.0);
        }

        #[test]
        fn test_zero_and_infinity_handling() {
            assert_eq!(numfmt_div_round(0.0, 5.0, NumfmtRoundMethod::Nearest), 0.0);
            assert_eq!(
                numfmt_div_round(1.0, 0.0, NumfmtRoundMethod::Nearest),
                f64::INFINITY
            );
        }

        #[test]
        fn test_negative_values_rounding() {
            assert_eq!(numfmt_div_round(-10.0, 3.0, NumfmtRoundMethod::Up), -3.3);
            assert_eq!(numfmt_div_round(-10.0, 3.0, NumfmtRoundMethod::Down), -3.4);
            assert_eq!(
                numfmt_div_round(-10.0, 3.0, NumfmtRoundMethod::Nearest),
                -3.3
            );
        }
    }

    #[cfg(test)]
    mod consider_suffix_tests {
        use super::*;
        use crate::units::{NumfmtRawSuffix::*, NumfmtWithI};

        fn setup_si_options() -> NumfmtUnit {
            NumfmtUnit::Si
        }

        fn setup_iec_options(with_i: NumfmtWithI) -> NumfmtUnit {
            NumfmtUnit::Iec(with_i)
        }

        #[test]
        fn test_no_conversion_needed() {
            let unit = NumfmtUnit::None;
            let result =
                numfmt_consider_suffix(1234.0, &unit, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(result, (1234.0, None));
        }

        #[test]
        fn test_si_conversion() {
            let unit = setup_si_options();
            let result = numfmt_consider_suffix(1e6, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, (1.0, Some((M, false))));
        }

        #[test]
        fn test_iec_conversion() {
            let unit = setup_iec_options(true);
            let result =
                numfmt_consider_suffix(1024.0 * 1024.0, &unit, NumfmtRoundMethod::Nearest, 0)
                    .unwrap();
            assert_eq!(result, (1.0, Some((M, true))));
        }

        #[test]
        fn test_auto_unit_error() {
            let unit = NumfmtUnit::Auto;
            let result = numfmt_consider_suffix(1234.0, &unit, NumfmtRoundMethod::Nearest, 2);
            assert!(result.is_err(), "Expected an error for unit 'auto'");
        }

        #[test]
        fn test_number_too_big() {
            let unit = setup_si_options();
            let result = numfmt_consider_suffix(1e30, &unit, NumfmtRoundMethod::Nearest, 0);
            assert!(result.is_err(), "Expected an error for too large number");
        }

        #[test]
        fn test_different_rounding_methods() {
            let unit = setup_si_options();
            let round_up =
                numfmt_consider_suffix(999_500.0, &unit, NumfmtRoundMethod::Up, 0).unwrap();
            assert_eq!(round_up, (1.0, Some((M, false))));

            let round_down =
                numfmt_consider_suffix(999_499.0, &unit, NumfmtRoundMethod::Down, 0).unwrap();
            assert_eq!(round_down, (999.0, Some((K, false))));
        }

        #[test]
        fn test_precision_control() {
            let unit = setup_si_options();
            let high_precision =
                numfmt_consider_suffix(1234567.0, &unit, NumfmtRoundMethod::Nearest, 3).unwrap();
            assert_eq!(high_precision, (1.235, Some((M, false))));
        }

        #[test]
        fn test_edge_cases_at_unit_boundaries() {
            let unit = setup_si_options();
            let edge_case =
                numfmt_consider_suffix(999_999.999, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(edge_case, (1.0, Some((M, false))));
        }

        #[test]
        fn test_multi_level_unit_thresholds() {
            let unit = setup_si_options();
            let result = numfmt_consider_suffix(1e9, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, (1.0, Some((G, false))));
        }

        #[test]
        fn test_iec_precision_interaction() {
            let unit = setup_iec_options(false);
            let result =
                numfmt_consider_suffix(1023.0 * 1024.0, &unit, NumfmtRoundMethod::Nearest, 3)
                    .unwrap();
            assert_eq!(result, (1023.0, Some((K, false))));
        }

        #[test]
        fn test_negative_values() {
            let unit = setup_si_options();
            let result =
                numfmt_consider_suffix(-1e6, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, (-1.0, Some((M, false))));
        }

        #[test]
        fn test_very_small_values_unit_conversion() {
            let unit = setup_si_options();
            let result =
                numfmt_consider_suffix(1e-6, &unit, NumfmtRoundMethod::Nearest, 9).unwrap();
            assert_eq!(result, (1e-6, None)); // Assuming Micro represents μ
        }

        #[test]
        fn test_unit_carry_over() {
            let unit = setup_si_options();
            let result =
                numfmt_consider_suffix(999_950.0, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, (1.0, Some((M, false))));
        }

        #[test]
        fn test_special_float_values() {
            let unit = setup_si_options();
            let result_nan = numfmt_consider_suffix(f64::NAN, &unit, NumfmtRoundMethod::Nearest, 2);
            assert!(result_nan.is_err(), "Expected an error for NaN input");

            let result_inf =
                numfmt_consider_suffix(f64::INFINITY, &unit, NumfmtRoundMethod::Nearest, 2);
            assert!(result_inf.is_err(), "Expected an error for Infinity input");
        }

        #[test]
        fn test_values_close_to_unit_thresholds() {
            let unit = setup_si_options();
            let result =
                numfmt_consider_suffix(999_999.5, &unit, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, (1.0, Some((M, false))));
        }
    }

    #[cfg(test)]
    mod transform_to_tests {
        use super::*;
        fn setup_transform_options() -> NumfmtTransformOptions {
            NumfmtTransformOptions {
                from: NumfmtUnit::None,
                from_unit: 1,
                to: NumfmtUnit::None,
                to_unit: 1,
            }
        }

        #[test]
        fn test_basic_no_conversion() {
            let opts = setup_transform_options();
            let result =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(result, "1234.57");
        }

        #[test]
        fn test_unit_conversion() {
            let mut opts = setup_transform_options();
            opts.from = NumfmtUnit::Si;
            opts.to = NumfmtUnit::Si;
            opts.to_unit = 1000; // Converting to kilounits
            let result =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(result, "0.00K");
        }

        #[test]
        fn test_rounding_methods() {
            let opts = setup_transform_options();
            let round_up = numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Up, 2).unwrap();
            assert_eq!(round_up, "1234.57");

            let round_down =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Down, 2).unwrap();
            assert_eq!(round_down, "1234.56");

            let round_nearest =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(round_nearest, "1235");
        }

        #[test]
        fn test_precision_control() {
            let opts = setup_transform_options();
            let high_precision =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 4).unwrap();
            assert_eq!(high_precision, "1234.5678");

            let no_precision =
                numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(no_precision, "1235");
        }

        #[test]
        fn test_edge_cases() {
            let opts = setup_transform_options();
            let zero_value =
                numfmt_transform_to(0.0, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(zero_value, "0.00");

            let very_small_value =
                numfmt_transform_to(0.0001234, &opts, NumfmtRoundMethod::Nearest, 6).unwrap();
            assert_eq!(very_small_value, "0.000123");

            let very_large_value =
                numfmt_transform_to(123456789.0, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(very_large_value, "123456789.00");
        }

        #[test]
        fn test_iec_true_to_si_conversion() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Iec(true),
                from_unit: 1024,
                to: NumfmtUnit::Si,
                to_unit: 1000,
            };
            let result = numfmt_transform_to(2048.0, &opts, NumfmtRoundMethod::Nearest, 1).unwrap();
            assert_eq!(result, "0.0K");
        }

        #[test]
        fn test_iec_false_to_si_conversion() {
            let opts = NumfmtTransformOptions {
                from: NumfmtUnit::Iec(false),
                from_unit: 1024,
                to: NumfmtUnit::Si,
                to_unit: 1000,
            };
            let result = numfmt_transform_to(2048.0, &opts, NumfmtRoundMethod::Nearest, 1).unwrap();
            assert_eq!(result, "0.0K");
        }

        #[test]
        fn test_decimal_and_negative_values() {
            let opts = setup_transform_options();
            let negative_value =
                numfmt_transform_to(-1234.5678, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(negative_value, "-1234.57");

            let decimal_value =
                numfmt_transform_to(0.98765, &opts, NumfmtRoundMethod::Nearest, 4).unwrap();
            assert_eq!(decimal_value, "0.9877");
        }

        #[test]
        fn test_very_large_values() {
            let opts = setup_transform_options();
            let large_value =
                numfmt_transform_to(1e12, &opts, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(large_value, "1000000000000");
        }

        #[test]
        fn test_non_standard_unit_conversion() {
            let mut opts = setup_transform_options();
            opts.to_unit = 500; // 非标准单位
            let result = numfmt_transform_to(1500.0, &opts, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, "3");
        }

        #[test]
        fn test_output_with_suffix() {
            let mut opts = setup_transform_options();
            opts.to = NumfmtUnit::Si;
            let result = numfmt_transform_to(1000.0, &opts, NumfmtRoundMethod::Nearest, 1).unwrap();
            assert_eq!(result, "1.0K"); // 假设这里是以k为后缀
        }

        #[test]
        fn test_non_numeric_input() {
            let opts = setup_transform_options();
            let result = numfmt_transform_to(
                "abc".parse().unwrap_or_default(),
                &opts,
                NumfmtRoundMethod::Nearest,
                2,
            );
            println!("{:?}", result);
            assert_eq!(result.unwrap(), "0.00");
        }

        #[test]
        fn test_extreme_decimal_precision() {
            let opts = setup_transform_options();
            let result =
                numfmt_transform_to(123.456, &opts, NumfmtRoundMethod::Nearest, 10).unwrap();
            assert_eq!(result, "123.4560000000");
        }

        #[test]
        fn test_extreme_unit_ratio_conversion() {
            let mut opts = setup_transform_options();
            opts.to_unit = 1_000_000; // 使用极端大的单位比例
            let result =
                numfmt_transform_to(1_000_000_000.0, &opts, NumfmtRoundMethod::Nearest, 0).unwrap();
            assert_eq!(result, "1000");
        }

        #[test]
        fn test_zero_input() {
            let opts = setup_transform_options();
            let result = numfmt_transform_to(0.0, &opts, NumfmtRoundMethod::Nearest, 2).unwrap();
            assert_eq!(result, "0.00");
        }

        #[test]
        fn test_very_small_input_values() {
            let opts = setup_transform_options();
            let result =
                numfmt_transform_to(0.000001234, &opts, NumfmtRoundMethod::Nearest, 10).unwrap();
            assert_eq!(result, "0.0000012340");
        }

        #[test]
        fn test_invalid_rounding_method() {
            let opts = setup_transform_options();
            let result = numfmt_transform_to(123.456, &opts, NumfmtRoundMethod::Up, 2);
            println!("{:?}", result);
            assert_eq!(result.unwrap(), "123.46");
        }

        #[test]
        fn test_input_with_special_characters() {
            let opts = setup_transform_options();
            let result = numfmt_transform_to(
                "12.34$".parse().unwrap_or_default(),
                &opts,
                NumfmtRoundMethod::Nearest,
                2,
            );
            println!("{:?}", result);
            assert_eq!(result.unwrap(), "0.00");
        }

        #[test]
        fn test_rounding_boundary_conditions() {
            let opts = setup_transform_options();
            let round_up_at_boundary =
                numfmt_transform_to(2.995, &opts, NumfmtRoundMethod::Up, 2).unwrap();
            assert_eq!(round_up_at_boundary, "3.00");

            let round_down_at_boundary =
                numfmt_transform_to(2.994, &opts, NumfmtRoundMethod::Down, 2).unwrap();
            assert_eq!(round_down_at_boundary, "2.99");
        }

        #[test]
        fn test_performance() {
            let opts = setup_transform_options();
            let start_time = std::time::Instant::now();
            for _ in 0..10000 {
                let _ = numfmt_transform_to(1234.5678, &opts, NumfmtRoundMethod::Nearest, 2);
            }
            let duration = start_time.elapsed();
            assert!(
                duration.as_secs_f64() < 1.0,
                "Performance test failed, took too long"
            );
        }
    }
    #[cfg(test)]
    mod format_string_tests {
        use super::*;
        use crate::NumfmtFormatOptions;
        #[test]
        fn test_format_string_basic() {
            let config = setup_default_config();
            let s = "123456789";
            let shell_expected_output = "124M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_field_selection() {
            let mut config = setup_default_config();
            config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }]; // 选择第二个字段
            let s = "123456789";
            let shell_expected_output = "124M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_basic_err() {
            let mut config = setup_default_config();
            config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }]; // 选择第二个字段
            let s = "123456789\n234567890";
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
        }

        #[test]
        fn test_format_string_padding_and_alignment() {
            let mut config = setup_default_config();
            config.padding = 10;
            let s = "123456789";
            let shell_expected_output = "124M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_padding_and_alignment_9() {
            let mut config = setup_default_config();
            config.padding = 9;
            let s = "1";
            let shell_expected_output = "1".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_auto_to_auto_err() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::Auto;
            let s = "123456789";
            let shell_expected_output = "Unit 'auto' isn't supported with --to options".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_none_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::Auto;
            let s = "123456789";
            let shell_expected_output = "Unit 'auto' isn't supported with --to options".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_iec_false_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::Auto;
            let s = "123456789";
            let shell_expected_output = "Unit 'auto' isn't supported with --to options".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_iec_true_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::Auto;
            let s = "123456789";
            let shell_expected_output =
                "missing 'i' suffix in input: '123456789' (e.g Ki/Mi/Gi)".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_si_true_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::Auto;
            let s = "123456789";
            let shell_expected_output = "Unit 'auto' isn't supported with --to options".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_auto_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096";
            let shell_expected_output = "102420484096".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_none_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096";
            let shell_expected_output = "102420484096".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_iec_false_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096";
            let shell_expected_output = "102420484096".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_iec_true_to_none_102420484096() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096";
            let shell_expected_output =
                "missing 'i' suffix in input: '102420484096' (e.g Ki/Mi/Gi)".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_iec_true_to_none_102420484096i() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096i";
            let shell_expected_output = "invalid suffix in input: '102420484096i'".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_iec_true_to_none_102420484096ki() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096Ki";
            let shell_expected_output = "104878575714304".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_si_to_none_102420484096ki() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096Ki";
            let shell_expected_output = "This suffix is unsupported for specified unit".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_si_to_none_102420484096k() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096K";
            let shell_expected_output = "102420484096000".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_si_to_none_102420484096() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::None;
            let s = "102420484096";
            let shell_expected_output = "102420484096".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_si_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::Si;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_transformations_from_iec_false_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::Si;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_iec_true_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::Si;
            let s = "102420484096";
            let shell_expected_output =
                "missing 'i' suffix in input: '102420484096' (e.g Ki/Mi/Gi)".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_none_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::Si;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_transformations_from_auto_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::Si;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_suffix_some_f() {
            let mut config = setup_default_config();
            config.suffix = Some("f".to_string());
            let s = "102420484096";
            let shell_expected_output = "103Gf".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_suffix_none() {
            let mut config = setup_default_config();
            config.suffix = None;
            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_default() {
            let mut config = setup_default_config();
            config.format = Default::default();

            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_config() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: None,
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "102420484096";
            let shell_expected_output = "+103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_config_grouping_true() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: true,
                padding: None,
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "102420484096";
            let shell_expected_output = "+103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_format_config_zero_padding_true() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: None,
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: true,
            };

            let s = "102420484096";
            let shell_expected_output = "+103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_config_padding_1() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: Some(1),
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "102420484096";
            let shell_expected_output = "+103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_config_padding_10() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: Some(10),
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "102420484096";
            let shell_expected_output = "+      103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_format_config_padding_10_zero_padding_true() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: Some(10),
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: true,
            };

            let s = "102420484096";
            let shell_expected_output = "+000000103G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_format_config_padding_10_precision_10() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: Some(10),
                precision: Some(10),
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "102420484096";
            let shell_expected_output = "+102.4204840960G-".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_header_0() {
            let mut config = setup_default_config();
            config.header = 0;

            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_header() {
            let mut config = setup_default_config();
            config.header = 2;

            let s = "102420484096";
            let shell_expected_output = "103G".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_rounding_down() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Down;
            let s = "102420484.096";
            let shell_expected_output = "102M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_rounding_from_zero() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::FromZero;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_rounding_towards_zero() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::TowardsZero;
            let s = "102420484.096";
            let shell_expected_output = "102M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_rounding_nearest() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Nearest;
            let s = "102420484.096";
            let shell_expected_output = "102M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_rounding_up() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Up;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }

        #[test]
        fn test_format_string_invalid_about() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Abort;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_invalid_fail() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Fail;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_invalid_warn() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Warn;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_invalid_ignore() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Ignore;
            let s = "102420484.096";
            let shell_expected_output = "103M".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_ok());
            assert_eq!(output.unwrap(), shell_expected_output);
        }
        #[test]
        fn test_format_string_null_handling() {
            let config = setup_default_config();
            let s = ""; // 非法输入
            let shell_expected_output = "invalid number: ''".to_string();
            let output = numfmt_format_string(s, &config, Some(1));
            println!("{:?}", output);
            assert!(output.is_err());
            assert_eq!(output.unwrap_err(), shell_expected_output);
        }
    }

    #[cfg(test)]
    mod format_and_print_delimited_tests {
        use super::*;
        #[test]
        fn test_basic_delimited() {
            let mut config = setup_default_config();
            config.delimiter = Some(" ".to_string());
            let s = "123 456 789";
            assert!(numfmt_format_and_print_delimited(s, &config).is_ok());
        }

        #[test]
        fn test_custom_delimiter() {
            let mut config = setup_default_config();
            config.delimiter = Some(",".to_string());
            let s = "100,200,300";
            assert!(numfmt_format_and_print_delimited(s, &config).is_ok());
        }

        #[test]
        fn test_field_selection() {
            let mut config = setup_default_config();
            config.delimiter = Some(" ".to_string());
            config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }]; // 选择第二个字段
            let s = "100 200 300";
            assert!(numfmt_format_and_print_delimited(s, &config).is_ok());
        }

        #[test]
        fn test_transformations() {
            let mut config = setup_default_config();
            config.delimiter = Some(" ".to_string());
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";
            assert!(numfmt_format_and_print_delimited(s, &config).is_ok());
        }

        #[test]
        fn test_edge_cases() {
            let mut config = setup_default_config();
            config.delimiter = Some(",".to_string());
            let s = ",100,,200,";
            assert!(numfmt_format_and_print_delimited(s, &config).is_ok());
        }
    }
    #[cfg(test)]
    mod format_and_print_whitespace_tests {
        use super::*;
        use crate::flags::NumfmtFormatOptions;
        #[test]
        fn test_basic() {
            let config = setup_default_config();
            let s = "123 456 789";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_field_selection() {
            let mut config = setup_default_config();
            config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }]; // 选择第二个字段
            let s = "123 456 789";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_basic2() {
            let mut config = setup_default_config();
            config.fields = vec![ctcore::ct_ranges::CtRange { low: 1, high: 1 }]; // 选择第二个字段
            let s = "123 456 789\n234 567 890";
            let _shell_expected_output = "";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_padding_and_alignment() {
            let mut config = setup_default_config();
            config.padding = 10;
            let s = "123 456 789";
            let _shell_expected_output = "       123       456       789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_padding_and_alignment_9() {
            let mut config = setup_default_config();
            config.padding = 9;
            let s = "1";
            let _shell_expected_output = "         1\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_auto_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::Auto;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_none_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::Auto;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_iec_false_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::Auto;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_iec_true_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::Auto;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_si_true_to_auto() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::Auto;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_auto_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::None;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_none_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::None;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_iec_false_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::None;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_iec_true_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::None;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_si_to_none() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::None;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_si_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Si;
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_transformations_from_iec_false_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(false);
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_iec_true_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Iec(true);
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_none_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::None;
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_transformations_from_auto_to_si() {
            let mut config = setup_default_config();
            config.transform.from = NumfmtUnit::Auto;
            config.transform.to = NumfmtUnit::Si;
            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_suffix_some_f() {
            let mut config = setup_default_config();
            config.suffix = Some("f".to_string());

            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_suffix_none() {
            let mut config = setup_default_config();
            config.suffix = None;

            let s = "1024 2048 4096";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_format_default() {
            let mut config = setup_default_config();
            config.format = Default::default();

            let s = "1024 2048 4096";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_format_config() {
            let mut config = setup_default_config();
            config.format = NumfmtFormatOptions {
                is_grouping: false,
                padding: None,
                precision: None,
                prefix: "+".to_string(),
                suffix: "-".to_string(),
                is_zero_padding: false,
            };

            let s = "1024 2048 4096";

            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_header_0() {
            let mut config = setup_default_config();
            config.header = 0;

            let s = "1024 2048 4096\n11111\n222222\n333333\n444444\n555555\n666666\n777777\n888888\n999999\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_header() {
            let mut config = setup_default_config();
            config.header = 2;

            let s = "1024 2048 4096\n11111\n222222\n333333\n444444\n555555\n666666\n777777\n888888\n999999\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_rounding_down() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Down;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_rounding_from_zero() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::FromZero;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_rounding_towards_zero() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::TowardsZero;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_rounding_nearest() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Nearest;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_rounding_up() {
            let mut config = setup_default_config();
            config.round = NumfmtRoundMethod::Up;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }

        #[test]
        fn test_invalid_about() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Abort;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_invalid_fail() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Fail;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_invalid_warn() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Warn;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_invalid_ignore() {
            let mut config = setup_default_config();
            config.invalid = NumfmtInvalidModes::Ignore;
            let s = "123.9 456.2 789.8";
            let _shell_expected_output = "123 456 789\n";
            let output = numfmt_format_and_print_whitespace(s, &config);
            assert!(output.is_ok());
        }
        #[test]
        fn test_null_handling() {
            let config = setup_default_config();
            let s = ""; // 非法输入
            assert!(numfmt_format_and_print_whitespace(s, &config).is_ok());
        }
    }
    #[cfg(test)]
    mod format_and_print_tests {
        use super::*;
        #[test]
        fn test_numfmt_format_and_print_with_delimiter() {
            let mut config = setup_default_config();
            config.delimiter = Some(','.to_string());

            assert!(numfmt_format_and_print("123456", &config).is_ok());
        }
        #[test]
        fn test_numfmt_format_and_print_without_delimiter() {
            let mut config = setup_default_config();
            config.delimiter = None;

            assert!(numfmt_format_and_print("123456", &config).is_ok());
        }
        // 测试错误情况，例如无效的配置
        #[test]
        fn test_numfmt_format_and_print_with_invalid_config() {
            let mut config = setup_default_config();
            config.delimiter = Some('\0'.to_string());

            assert!(numfmt_format_and_print("123456", &config).is_ok());
        }
        // 测试空字符串
        #[test]
        fn test_numfmt_format_and_print_with_empty_string() {
            let mut config = setup_default_config();
            config.delimiter = Some('\t'.to_string());

            assert!(numfmt_format_and_print("", &config).is_ok());
        }
        // 测试大数字
        #[test]
        fn test_numfmt_format_and_print_large_number() {
            let mut config = setup_default_config();
            config.delimiter = Some('.'.to_string());
            assert!(numfmt_format_and_print("1234567890", &config).is_ok());
        }
    }

    #[cfg(test)]
    mod round_with_precision_tests {
        use super::*;
        #[test]
        fn test_numfmt_round_with_precision() {
            // Test case 1: Round down with precision 0
            let result1 = numfmt_round_with_precision(3.14159, NumfmtRoundMethod::Down, 0);
            assert_eq!(result1, 3.0);

            // Test case 2: Round up with precision 2
            let result2 = numfmt_round_with_precision(3.14159, NumfmtRoundMethod::Up, 2);
            assert_eq!(result2, 3.15);

            // Test case 3: Round to nearest with precision 1
            let result3 = numfmt_round_with_precision(3.14159, NumfmtRoundMethod::Nearest, 1);
            assert_eq!(result3, 3.1);
        }
        #[test]
        #[allow(clippy::cognitive_complexity)]
        fn test_base_round_with_precision() {
            let rm = NumfmtRoundMethod::FromZero;
            assert_eq!(1.0, numfmt_round_with_precision(0.12345, rm, 0));
            assert_eq!(0.2, numfmt_round_with_precision(0.12345, rm, 1));
            assert_eq!(0.13, numfmt_round_with_precision(0.12345, rm, 2));
            assert_eq!(0.124, numfmt_round_with_precision(0.12345, rm, 3));
            assert_eq!(0.1235, numfmt_round_with_precision(0.12345, rm, 4));
            assert_eq!(0.12345, numfmt_round_with_precision(0.12345, rm, 5));

            let rm = NumfmtRoundMethod::TowardsZero;
            assert_eq!(0.0, numfmt_round_with_precision(0.12345, rm, 0));
            assert_eq!(0.1, numfmt_round_with_precision(0.12345, rm, 1));
            assert_eq!(0.12, numfmt_round_with_precision(0.12345, rm, 2));
            assert_eq!(0.123, numfmt_round_with_precision(0.12345, rm, 3));
            assert_eq!(0.1234, numfmt_round_with_precision(0.12345, rm, 4));
            assert_eq!(0.12345, numfmt_round_with_precision(0.12345, rm, 5));
        }

        #[test]
        fn test_base_parse_implicit_precision() {
            assert_eq!(0, numfmt_parse_implicit_precision(""));
            assert_eq!(0, numfmt_parse_implicit_precision("1"));
            assert_eq!(1, numfmt_parse_implicit_precision("1.2"));
            assert_eq!(2, numfmt_parse_implicit_precision("1.23"));
            assert_eq!(3, numfmt_parse_implicit_precision("1.234"));
            assert_eq!(0, numfmt_parse_implicit_precision("1K"));
            assert_eq!(1, numfmt_parse_implicit_precision("1.2K"));
            assert_eq!(2, numfmt_parse_implicit_precision("1.23K"));
            assert_eq!(3, numfmt_parse_implicit_precision("1.234K"));
        }
    }
}