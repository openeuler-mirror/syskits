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

use ctcore::ct_display::Quotable;

use crate::formatteriteminfo::FormatterItemInfo;
use crate::prn_format::*;

/// 解析后的格式化器信息
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ParsedFormatterItemInfo {
    /// 格式化器的基本信息（包含函数指针和格式化参数）
    pub formatter_item_info: FormatterItemInfo,
    /// 是否在输出中添加ASCII转储
    pub add_ascii_dump: bool,
}

impl ParsedFormatterItemInfo {
    /// 创建新的格式化器信息实例
    pub fn new(formatter_item_info: FormatterItemInfo, add_ascii_dump: bool) -> Self {
        Self {
            formatter_item_info,
            add_ascii_dump,
        }
    }
}

/// 处理传统格式的参数字符
///
/// # 参数
/// * `ch` - 格式字符
///
/// # 返回值
/// * `Option<FormatterItemInfo>` - 对应的格式化器信息，如果字符无效则返回 None
fn od_argument_traditional_format(ch: char) -> Option<FormatterItemInfo> {
    match ch {
        'a' => Some(FORMAT_ITEM_A),      // ASCII 格式
        'B' => Some(FORMAT_ITEM_OCT16),  // 16位八进制
        'b' => Some(FORMAT_ITEM_OCT8),   // 8位八进制
        'c' => Some(FORMAT_ITEM_C),      // 字符格式
        'D' => Some(FORMAT_ITEM_DEC32U), // 32位无符号十进制
        'd' => Some(FORMAT_ITEM_DEC16U), // 16位无符号十进制
        'e' => Some(FORMAT_ITEM_F64),    // 64位浮点数
        'F' => Some(FORMAT_ITEM_F64),    // 64位浮点数
        'f' => Some(FORMAT_ITEM_F32),    // 32位浮点数
        'H' => Some(FORMAT_ITEM_HEX32),  // 32位十六进制
        'h' => Some(FORMAT_ITEM_HEX16),  // 16位十六进制
        'i' => Some(FORMAT_ITEM_DEC32S), // 32位有符号十进制
        'I' => Some(FORMAT_ITEM_DEC64S), // 64位有符号十进制
        'L' => Some(FORMAT_ITEM_DEC64S), // 64位有符号十进制
        'l' => Some(FORMAT_ITEM_DEC64S), // 64位有符号十进制
        'O' => Some(FORMAT_ITEM_OCT32),  // 32位八进制
        'o' => Some(FORMAT_ITEM_OCT16),  // 16位八进制
        's' => Some(FORMAT_ITEM_DEC16S), // 16位有符号十进制
        'X' => Some(FORMAT_ITEM_HEX32),  // 32位十六进制
        'x' => Some(FORMAT_ITEM_HEX16),  // 16位十六进制
        _ => None,
    }
}

/// 根据类型和字节大小获取对应的格式化器
///
/// # 参数
/// * `type_char` - 格式类型
/// * `byte_size` - 字节大小
///
/// # 返回值
/// * `Option<FormatterItemInfo>` - 对应的格式化器信息，如果组合无效则返回 None
fn od_format_type(type_char: FormatType, byte_size: u8) -> Option<FormatterItemInfo> {
    match (type_char, byte_size) {
        // ASCII 和字符格式不依赖字节大小
        (FormatType::Ascii, _) => Some(FORMAT_ITEM_A),
        (FormatType::Char, _) => Some(FORMAT_ITEM_C),

        // 有符号十进制整数
        (FormatType::DecimalInt, 1) => Some(FORMAT_ITEM_DEC8S),
        (FormatType::DecimalInt, 2) => Some(FORMAT_ITEM_DEC16S),
        (FormatType::DecimalInt, 0 | 4) => Some(FORMAT_ITEM_DEC32S),
        (FormatType::DecimalInt, 8) => Some(FORMAT_ITEM_DEC64S),

        // 八进制整数
        (FormatType::OctalInt, 1) => Some(FORMAT_ITEM_OCT8),
        (FormatType::OctalInt, 2) => Some(FORMAT_ITEM_OCT16),
        (FormatType::OctalInt, 0 | 4) => Some(FORMAT_ITEM_OCT32),
        (FormatType::OctalInt, 8) => Some(FORMAT_ITEM_OCT64),

        // 无符号十进制整数
        (FormatType::UnsignedInt, 1) => Some(FORMAT_ITEM_DEC8U),
        (FormatType::UnsignedInt, 2) => Some(FORMAT_ITEM_DEC16U),
        (FormatType::UnsignedInt, 0 | 4) => Some(FORMAT_ITEM_DEC32U),
        (FormatType::UnsignedInt, 8) => Some(FORMAT_ITEM_DEC64U),

        // 十六进制整数
        (FormatType::HexadecimalInt, 1) => Some(FORMAT_ITEM_HEX8),
        (FormatType::HexadecimalInt, 2) => Some(FORMAT_ITEM_HEX16),
        (FormatType::HexadecimalInt, 0 | 4) => Some(FORMAT_ITEM_HEX32),
        (FormatType::HexadecimalInt, 8) => Some(FORMAT_ITEM_HEX64),

        // 浮点数
        (FormatType::Float, 2) => Some(FORMAT_ITEM_F16),
        (FormatType::Float, 0 | 4) => Some(FORMAT_ITEM_F32),
        (FormatType::Float, 8) => Some(FORMAT_ITEM_F64),

        _ => None,
    }
}

/// 检查字符是否需要额外的选项参数
fn od_argument_with_option(ch: char) -> bool {
    matches!(ch, 'A' | 'j' | 'N' | 'S' | 'w')
}

/// 从命令行参数解析格式标志
///
/// 标准的参数解析库（如 getopts、docopt、clap）不太适合解析格式标志，因为：
/// 1. 参数可以多次出现，且顺序很重要
/// 2. 参数可以单独使用或组合使用（如 -f、-o、-x 可以写成 -fox）
/// 3. 可以与非格式相关的标志混合（如 -v：-fvox）
/// 4. 带参数的标志（如 -w16）只能出现在末尾
/// 5. -t/--format 的参数可以指定一个或多个格式
/// 6. 如果遇到 -- 则停止解析
#[allow(clippy::cognitive_complexity)]
pub fn od_parse_format_flags(args: &[String]) -> Result<Vec<ParsedFormatterItemInfo>, String> {
    let mut formats = Vec::new();
    let mut expect_type_string = false;

    // 跳过程序名称(args[0])，处理剩余参数
    for arg in args.iter().skip(1) {
        if arg == "--" {
            // 遇到 -- 停止解析
            break;
        }

        if expect_type_string {
            // 处理预期的类型字符串
            handle_type_string(arg, &mut formats)?;
            expect_type_string = false;
        } else {
            // 处理命令行参数
            expect_type_string = handle_argument(arg, &mut formats, expect_type_string)?;
        }
    }

    // 检查是否缺少格式说明符
    if expect_type_string {
        return Err("missing format specification after '--format' / '-t'".to_string());
    }

    // 如果没有指定格式，使用默认格式
    if formats.is_empty() {
        formats.push(ParsedFormatterItemInfo::new(FORMAT_ITEM_OCT16, false));
    }

    Ok(formats)
}

/// 处理单个命令行参数
fn handle_argument(
    arg: &str,
    formats: &mut Vec<ParsedFormatterItemInfo>,
    mut expect_type_string: bool,
) -> Result<bool, String> {
    if arg.starts_with("--") {
        handle_long_option(arg, formats, &mut expect_type_string)?;
    } else if arg.starts_with('-') {
        handle_short_option(arg, formats, &mut expect_type_string)?;
    }
    Ok(expect_type_string)
}

/// 处理长选项（以--开头的参数）
fn handle_long_option(
    arg: &str,
    formats: &mut Vec<ParsedFormatterItemInfo>,
    expect_type_string: &mut bool,
) -> Result<(), String> {
    // 处理 --format=value 形式
    if arg.starts_with("--format=") {
        let params: String = arg.chars().skip_while(|c| *c != '=').skip(1).collect();
        let v = od_parse_type_string(&params)?;
        formats.extend(v);
    }
    // 处理 --format 形式
    else if arg == "--format" {
        *expect_type_string = true;
    }
    // 其他 -- 开头的参数忽略

    Ok(())
}

/// 处理短选项（以-开头的参数）
fn handle_short_option(
    arg: &str,
    formats: &mut Vec<ParsedFormatterItemInfo>,
    expect_type_string: &mut bool,
) -> Result<(), String> {
    let mut format_spec = String::new();

    // 遍历参数中的字符
    for c in arg.chars().skip(1) {
        if *expect_type_string {
            // 如果期待类型字符串，收集剩余字符
            format_spec.push(c);
        } else if od_argument_with_option(c) {
            // 如果是需要选项的参数，停止处理
            break;
        } else if c == 't' {
            // 设置期待类型字符串标志
            *expect_type_string = true;
        } else if let Some(r) = od_argument_traditional_format(c) {
            // 处理传统格式参数
            formats.push(ParsedFormatterItemInfo::new(r, false));
        }
    }

    // 如果收集到格式说明，解析它
    if !format_spec.is_empty() {
        let v = od_parse_type_string(&format_spec)?;
        formats.extend(v);
        *expect_type_string = false;
    }

    Ok(())
}

/// 处理类型字符串
fn handle_type_string(
    type_str: &str,
    formats: &mut Vec<ParsedFormatterItemInfo>,
) -> Result<(), String> {
    let v = od_parse_type_string(type_str)?;
    formats.extend(v);
    Ok(())
}

fn is_format_size_char(
    ch: Option<char>,
    format_type: FormatTypeCategory,
    byte_size: &mut u8,
) -> bool {
    match (format_type, ch) {
        (FormatTypeCategory::Integer, Some('C')) => {
            *byte_size = 1;
            true
        }
        (FormatTypeCategory::Integer, Some('S')) => {
            *byte_size = 2;
            true
        }
        (FormatTypeCategory::Integer, Some('I')) => {
            *byte_size = 4;
            true
        }
        (FormatTypeCategory::Integer, Some('L')) => {
            *byte_size = 8;
            true
        }

        (FormatTypeCategory::Float, Some('F')) => {
            *byte_size = 4;
            true
        }
        (FormatTypeCategory::Float, Some('D')) => {
            *byte_size = 8;
            true
        }
        // FormatTypeCategory::Float, 'L' => *byte_size = 16, // TODO support f128
        _ => false,
    }
}

fn is_format_size_decimal(
    ch: Option<char>,
    format_type: FormatTypeCategory,
    decimal_size: &mut String,
) -> bool {
    if format_type == FormatTypeCategory::Char {
        return false;
    }
    match ch {
        Some(d) if d.is_ascii_digit() => {
            decimal_size.push(d);
            true
        }
        _ => false,
    }
}

fn is_format_dump_char(ch: Option<char>, show_ascii_dump: &mut bool) -> bool {
    match ch {
        Some('z') => {
            *show_ascii_dump = true;
            true
        }
        _ => false,
    }
}
/// 解析类型字符串，将其转换为格式化器信息列表
///
/// # 参数
/// * `params` - 要解析的类型字符串，例如 "x4z"、"d8"、"f"
///
/// # 返回值
/// * `Result<Vec<ParsedFormatterItemInfo>, String>` - 成功则返回格式化器列表，失败则返回错误信息
fn od_parse_type_string(params: &str) -> Result<Vec<ParsedFormatterItemInfo>, String> {
    // 存储解析出的所有格式化器
    let mut formats = Vec::new();

    // 创建字符迭代器
    let mut chars = params.chars();
    let mut ch = chars.next();

    // 循环处理每个格式说明符
    while let Some(type_char) = ch {
        // 解析格式类型字符（如 'x'、'd'、'f' 等）
        let type_char = format_type(type_char).ok_or_else(|| {
            format!(
                "unexpected char '{}' in format specification {}",
                type_char,
                params.quote()
            )
        })?;

        // 获取格式类型的类别（整数、浮点数、字符等）
        let type_cat = format_type_category(type_char);

        // 获取下一个字符
        ch = chars.next();

        // 解析大小规格和 ASCII 转储标志
        let mut byte_size = 0u8;
        let mut show_ascii_dump = false;

        // 检查是否有大小字符（如 'C'、'S'、'I'、'L'）
        if is_format_size_char(ch, type_cat, &mut byte_size) {
            ch = chars.next();
        } else {
            // 如果没有大小字符，尝试解析数字大小
            let mut decimal_size = String::new();
            while is_format_size_decimal(ch, type_cat, &mut decimal_size) {
                ch = chars.next();
            }
            // 如果有数字大小，将其转换为字节数
            if !decimal_size.is_empty() {
                byte_size = decimal_size.parse().map_err(|_| {
                    format!(
                        "invalid number {} in format specification {}",
                        decimal_size.quote(),
                        params.quote()
                    )
                })?;
            }
        }

        // 检查是否有 ASCII 转储标志 ('z')
        if is_format_dump_char(ch, &mut show_ascii_dump) {
            ch = chars.next();
        }

        // 根据类型和大小获取对应的格式化器
        let ft = od_format_type(type_char, byte_size).ok_or_else(|| {
            format!(
                "invalid size '{}' in format specification {}",
                byte_size,
                params.quote()
            )
        })?;

        // 将解析好的格式化器添加到结果列表
        formats.push(ParsedFormatterItemInfo::new(ft, show_ascii_dump));
    }

    Ok(formats)
}

/// 格式类型枚举
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
enum FormatType {
    Ascii,          // ASCII 格式
    Char,           // 字符格式
    DecimalInt,     // 十进制整数
    OctalInt,       // 八进制整数
    UnsignedInt,    // 无符号整数
    HexadecimalInt, // 十六进制整数
    Float,          // 浮点数
}

/// 格式类型的分类
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
enum FormatTypeCategory {
    Char,    // 字符类型
    Integer, // 整数类型
    Float,   // 浮点类型
}

fn format_type(ch: char) -> Option<FormatType> {
    match ch {
        'a' => Some(FormatType::Ascii),
        'c' => Some(FormatType::Char),
        'd' => Some(FormatType::DecimalInt),
        'o' => Some(FormatType::OctalInt),
        'u' => Some(FormatType::UnsignedInt),
        'x' => Some(FormatType::HexadecimalInt),
        'f' => Some(FormatType::Float),
        _ => None,
    }
}

fn format_type_category(t: FormatType) -> FormatTypeCategory {
    match t {
        FormatType::Ascii | FormatType::Char => FormatTypeCategory::Char,
        FormatType::DecimalInt
        | FormatType::OctalInt
        | FormatType::UnsignedInt
        | FormatType::HexadecimalInt => FormatTypeCategory::Integer,
        FormatType::Float => FormatTypeCategory::Float,
    }
}

#[cfg(test)]
pub fn parse_format_flags_str(args_str: &[&'static str]) -> Result<Vec<FormatterItemInfo>, String> {
    let args: Vec<String> = args_str.iter().map(|s| s.to_string()).collect();
    od_parse_format_flags(&args).map(|v| {
        // tests using this function assume add_ascii_dump is not set
        v.into_iter()
            .inspect(|f| assert!(!f.add_ascii_dump))
            .map(|f| f.formatter_item_info)
            .collect()
    })
}

#[test]
fn test_no_options() {
    assert_eq!(
        parse_format_flags_str(&["od"]).unwrap(),
        vec![FORMAT_ITEM_OCT16]
    );
}

#[test]
fn test_one_option() {
    assert_eq!(
        parse_format_flags_str(&["od", "-F"]).unwrap(),
        vec![FORMAT_ITEM_F64]
    );
}

#[test]
fn test_two_separate_options() {
    assert_eq!(
        parse_format_flags_str(&["od", "-F", "-x"]).unwrap(),
        vec![FORMAT_ITEM_F64, FORMAT_ITEM_HEX16]
    );
}

#[test]
fn test_two_combined_options() {
    assert_eq!(
        parse_format_flags_str(&["od", "-Fx"]).unwrap(),
        vec![FORMAT_ITEM_F64, FORMAT_ITEM_HEX16]
    );
}

#[test]
fn test_ignore_non_format_parameters() {
    assert_eq!(
        parse_format_flags_str(&["od", "-d", "-Ax"]).unwrap(),
        vec![FORMAT_ITEM_DEC16U]
    );
}

#[test]
fn test_ignore_separate_parameters() {
    assert_eq!(
        parse_format_flags_str(&["od", "-I", "-A", "x"]).unwrap(),
        vec![FORMAT_ITEM_DEC64S]
    );
}

#[test]
fn test_ignore_trailing_vals() {
    assert_eq!(
        parse_format_flags_str(&["od", "-D", "--", "-x"]).unwrap(),
        vec![FORMAT_ITEM_DEC32U]
    );
}

#[test]
fn test_invalid_long_format() {
    parse_format_flags_str(&["od", "--format=X"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=xX"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=aC"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=fI"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=xD"]).unwrap_err();

    parse_format_flags_str(&["od", "--format=xC1"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=x1C"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=xz1"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=xzC"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=xzz"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=xCC"]).unwrap_err();

    parse_format_flags_str(&["od", "--format=c1"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=x256"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=d5"]).unwrap_err();
    parse_format_flags_str(&["od", "--format=f1"]).unwrap_err();
}

#[test]
fn test_long_format_a() {
    assert_eq!(
        parse_format_flags_str(&["od", "--format=a"]).unwrap(),
        vec![FORMAT_ITEM_A]
    );
}

#[test]
fn test_long_format_cz() {
    assert_eq!(
        od_parse_format_flags(&["od".to_string(), "--format=cz".to_string()]).unwrap(),
        vec![ParsedFormatterItemInfo::new(FORMAT_ITEM_C, true)]
    );
}

#[test]
fn test_long_format_d() {
    assert_eq!(
        parse_format_flags_str(&["od", "--format=d8"]).unwrap(),
        vec![FORMAT_ITEM_DEC64S]
    );
}

#[test]
fn test_long_format_d_default() {
    assert_eq!(
        parse_format_flags_str(&["od", "--format=d"]).unwrap(),
        vec![FORMAT_ITEM_DEC32S]
    );
}

#[test]
fn test_long_format_o_default() {
    assert_eq!(
        parse_format_flags_str(&["od", "--format=o"]).unwrap(),
        vec![FORMAT_ITEM_OCT32]
    );
}

#[test]
fn test_long_format_u_default() {
    assert_eq!(
        parse_format_flags_str(&["od", "--format=u"]).unwrap(),
        vec![FORMAT_ITEM_DEC32U]
    );
}

#[test]
fn test_long_format_x_default() {
    assert_eq!(
        parse_format_flags_str(&["od", "--format=x"]).unwrap(),
        vec![FORMAT_ITEM_HEX32]
    );
}

#[test]
fn test_long_format_f_default() {
    assert_eq!(
        parse_format_flags_str(&["od", "--format=f"]).unwrap(),
        vec![FORMAT_ITEM_F32]
    );
}

#[test]
fn test_long_format_next_arg() {
    assert_eq!(
        parse_format_flags_str(&["od", "--format", "f8"]).unwrap(),
        vec![FORMAT_ITEM_F64]
    );
}

#[test]
fn test_short_format_next_arg() {
    assert_eq!(
        parse_format_flags_str(&["od", "-t", "x8"]).unwrap(),
        vec![FORMAT_ITEM_HEX64]
    );
}

#[test]
fn test_short_format_combined_arg() {
    assert_eq!(
        parse_format_flags_str(&["od", "-tu8"]).unwrap(),
        vec![FORMAT_ITEM_DEC64U]
    );
}

#[test]
fn test_format_next_arg_invalid() {
    parse_format_flags_str(&["od", "--format", "-v"]).unwrap_err();
    parse_format_flags_str(&["od", "--format"]).unwrap_err();
    parse_format_flags_str(&["od", "-t", "-v"]).unwrap_err();
    parse_format_flags_str(&["od", "-t"]).unwrap_err();
}

#[test]
fn test_mixed_formats() {
    assert_eq!(
        od_parse_format_flags(&[
            "od".to_string(),
            "--skip-bytes=2".to_string(),
            "-vItu1z".to_string(),
            "-N".to_string(),
            "1000".to_string(),
            "-xt".to_string(),
            "acdx1".to_string(),
            "--format=u2c".to_string(),
            "--format".to_string(),
            "f".to_string(),
            "-xAx".to_string(),
            "--".to_string(),
            "-h".to_string(),
            "--format=f8".to_string(),
        ])
        .unwrap(),
        vec![
            ParsedFormatterItemInfo::new(FORMAT_ITEM_DEC64S, false), // I
            ParsedFormatterItemInfo::new(FORMAT_ITEM_DEC8U, true),   // tu1z
            ParsedFormatterItemInfo::new(FORMAT_ITEM_HEX16, false),  // x
            ParsedFormatterItemInfo::new(FORMAT_ITEM_A, false),      // ta
            ParsedFormatterItemInfo::new(FORMAT_ITEM_C, false),      // tc
            ParsedFormatterItemInfo::new(FORMAT_ITEM_DEC32S, false), // td
            ParsedFormatterItemInfo::new(FORMAT_ITEM_HEX8, false),   // tx1
            ParsedFormatterItemInfo::new(FORMAT_ITEM_DEC16U, false), // tu2
            ParsedFormatterItemInfo::new(FORMAT_ITEM_C, false),      // tc
            ParsedFormatterItemInfo::new(FORMAT_ITEM_F32, false),    // tf
            ParsedFormatterItemInfo::new(FORMAT_ITEM_HEX16, false),  // x
        ]
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_type_string() {
        // 测试基本格式
        let result = od_parse_type_string("x").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].formatter_item_info, FORMAT_ITEM_HEX32);
        assert!(!result[0].add_ascii_dump);

        // 测试带字节大小的格式
        let result = od_parse_type_string("x4").unwrap();
        assert_eq!(result[0].formatter_item_info, FORMAT_ITEM_HEX32);

        // 测试带ASCII转储的格式
        let result = od_parse_type_string("x4z").unwrap();
        assert_eq!(result[0].formatter_item_info, FORMAT_ITEM_HEX32);
        assert!(result[0].add_ascii_dump);

        // 测试多个格式组合
        let result = od_parse_type_string("x4zd2").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].formatter_item_info, FORMAT_ITEM_HEX32);
        assert!(result[0].add_ascii_dump);
        assert_eq!(result[1].formatter_item_info, FORMAT_ITEM_DEC16S);
    }

    #[test]
    fn test_parse_type_string_errors() {
        // 测试无效的格式字符
        assert!(od_parse_type_string("y").is_err());

        // 测试无效的大小
        assert!(od_parse_type_string("x9").is_err());

        // 测试无效的格式组合
        assert!(od_parse_type_string("xC1").is_err());
        assert!(od_parse_type_string("x1C").is_err());
    }

    #[test]
    fn test_od_parse_format_flags_complex() {
        // 测试复杂的命令行参数组合
        let args = vec![
            "od".to_string(),
            "-t".to_string(),
            "x2z".to_string(),            // 16位十六进制，带ASCII转储
            "--format=d4".to_string(), // 32位十进制
            "-F".to_string(),             // 64位浮点数
        ];

        let result = od_parse_format_flags(&args).unwrap();
        assert_eq!(result.len(), 3);

        // 验证第一个格式：16位十六进制带ASCII转储
        assert_eq!(result[0].formatter_item_info, FORMAT_ITEM_HEX16);
        assert!(result[0].add_ascii_dump);

        // 验证第二个格式：32位十进制
        assert_eq!(result[1].formatter_item_info, FORMAT_ITEM_DEC32S);
        assert!(!result[1].add_ascii_dump);

        // 验证第三个格式：64位浮点数
        assert_eq!(result[2].formatter_item_info, FORMAT_ITEM_F64);
        assert!(!result[2].add_ascii_dump);
    }

    #[test]
    fn test_od_parse_format_flags_edge_cases() {
        // 测试空参数列表
        let result = od_parse_format_flags(&["od".to_string()]).unwrap();
        println!("Empty args result: {:?}", result);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].formatter_item_info, FORMAT_ITEM_OCT16); // 默认格式

        // 测试 -- 后的参数被忽略
        let args = vec![
            "od".to_string(),
            "-x".to_string(),
            "--".to_string(),
            "-d".to_string(),
        ];
        println!("Args before processing: {:?}", args);
        let result = od_parse_format_flags(&args).unwrap();
        println!("Result after --: {:?}", result);
        for (i, fmt) in result.iter().enumerate() {
            println!(
                "Format {}: {:?}, ascii_dump: {}",
                i, fmt.formatter_item_info, fmt.add_ascii_dump
            );
        }
        assert_eq!(result.len(), 1, "Expected 1 format, got {}", result.len());
        assert_eq!(result[0].formatter_item_info, FORMAT_ITEM_HEX16);

        // 测试缺少格式说明符
        let args = vec!["od".to_string(), "-t".to_string()];
        let err = od_parse_format_flags(&args).unwrap_err();
        println!("Expected error for missing format: {}", err);
        assert!(err.contains("missing format specification"));
    }
}
