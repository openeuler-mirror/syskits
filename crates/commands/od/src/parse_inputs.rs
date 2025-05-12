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
use super::od_options;
use clap::ArgMatches;

/// 命令行选项抽象接口
pub trait CommandLineOpts {
    /// 返回所有不属于选项的命令行参数
    fn inputs(&self) -> Vec<&str>;
    /// 测试指定的选项是否存在
    fn opts_present(&self, _: &[&str]) -> bool;
}

/// 为 `ArgMatches` 实现 CommandLineOpts 接口
impl CommandLineOpts for ArgMatches {
    fn inputs(&self) -> Vec<&str> {
        self.get_many::<String>(od_options::OD_FILENAME)
            .map(|values| values.map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    fn opts_present(&self, opts: &[&str]) -> bool {
        opts.iter()
            .any(|opt| self.value_source(opt) == Some(clap::parser::ValueSource::CommandLine))
    }
}

/// 包含输入文件名和可选的偏移量
///
/// `FileNames` 用于一个或多个文件输入（"-" 表示标准输入）
/// `FileAndOffset` 用于单个文件输入，带有偏移量和可选的标签。
/// 偏移量和标签以字节为单位指定。
/// 只有在指定了偏移量时才会使用 `FileAndOffset`，
/// 但偏移量可能为0。
#[derive(PartialEq, Eq, Debug)]
pub enum CommandLineInputs {
    FileNames(Vec<String>),
    FileAndOffset((String, u64, Option<u64>)),
}

/// 解析 od 的命令行输入
pub fn od_parse_inputs(matches: &dyn CommandLineOpts) -> Result<CommandLineInputs, String> {
    let mut input_strings = matches.inputs();

    // 如果使用了 --traditional 选项，使用传统解析方式
    if matches.opts_present(&["traditional"]) {
        return od_parse_inputs_traditional(&input_strings);
    }

    // 尝试解析偏移量
    if let Some(result) = try_parse_offset(matches, &input_strings) {
        return Ok(result);
    }

    // 如果没有输入，使用标准输入
    if input_strings.is_empty() {
        input_strings.push("-");
    }

    Ok(CommandLineInputs::FileNames(
        input_strings.iter().map(|&s| s.to_string()).collect(),
    ))
}

/// 尝试解析偏移量
fn try_parse_offset(
    matches: &dyn CommandLineOpts,
    input_strings: &[&str],
) -> Option<CommandLineInputs> {
    // 只有在有1-2个参数时才尝试解析偏移量
    if input_strings.len() != 1 && input_strings.len() != 2 {
        return None;
    }

    // 如果存在特定选项，则不解析偏移量
    if has_offset_incompatible_options(matches) {
        return None;
    }

    // 获取最后一个参数并尝试解析为偏移量
    let last_arg = input_strings[input_strings.len() - 1];
    if let Ok(offset) = parse_offset_operand(last_arg) {
        return create_offset_input(input_strings, offset);
    }

    None
}

/// 检查是否存在与偏移量不兼容的选项
fn has_offset_incompatible_options(matches: &dyn CommandLineOpts) -> bool {
    matches.opts_present(&[
        od_options::OD_ADDRESS_RADIX,
        od_options::OD_READ_BYTES,
        od_options::OD_SKIP_BYTES,
        od_options::OD_FORMAT,
        od_options::OD_OUTPUT_DUPLICATES,
        od_options::OD_WIDTH,
    ])
}

/// 根据输入参数和偏移量创建 CommandLineInputs
fn create_offset_input(input_strings: &[&str], offset: u64) -> Option<CommandLineInputs> {
    match input_strings.len() {
        1 if input_strings[0].starts_with('+') => Some(CommandLineInputs::FileAndOffset((
            "-".to_string(),
            offset,
            None,
        ))),
        2 => Some(CommandLineInputs::FileAndOffset((
            input_strings[0].to_string(),
            offset,
            None,
        ))),
        _ => None,
    }
}

/// 当命令行包含 --traditional 时解析输入
pub fn od_parse_inputs_traditional(input_strings: &[&str]) -> Result<CommandLineInputs, String> {
    match input_strings.len() {
        0 => Ok(create_stdin_input()),
        1 => parse_traditional_single_arg(input_strings),
        2 => parse_traditional_two_args(input_strings),
        3 => parse_traditional_three_args(input_strings),
        _ => Err(format!(
            "too many inputs after --traditional: {}",
            input_strings[3]
        )),
    }
}

/// 创建标准输入的 CommandLineInputs
fn create_stdin_input() -> CommandLineInputs {
    CommandLineInputs::FileNames(vec!["-".to_string()])
}

/// 解析传统模式下的单个参数
fn parse_traditional_single_arg(input_strings: &[&str]) -> Result<CommandLineInputs, String> {
    let offset = parse_offset_operand(input_strings[0]);
    Ok(match offset {
        Ok(n) => CommandLineInputs::FileAndOffset(("-".to_string(), n, None)),
        _ => CommandLineInputs::FileNames(vec![input_strings[0].to_string()]),
    })
}

/// 解析传统模式下的两个参数
fn parse_traditional_two_args(input_strings: &[&str]) -> Result<CommandLineInputs, String> {
    let offset0 = parse_offset_operand(input_strings[0]);
    let offset1 = parse_offset_operand(input_strings[1]);

    match (offset0, offset1) {
        (Ok(n), Ok(m)) => Ok(CommandLineInputs::FileAndOffset((
            "-".to_string(),
            n,
            Some(m),
        ))),
        (_, Ok(m)) => Ok(CommandLineInputs::FileAndOffset((
            input_strings[0].to_string(),
            m,
            None,
        ))),
        _ => Err(format!("invalid offset: {}", input_strings[1])),
    }
}

/// 解析传统模式下的三个参数
fn parse_traditional_three_args(input_strings: &[&str]) -> Result<CommandLineInputs, String> {
    let offset = parse_offset_operand(input_strings[1]);
    let label = parse_offset_operand(input_strings[2]);

    match (offset, label) {
        (Ok(n), Ok(m)) => Ok(CommandLineInputs::FileAndOffset((
            input_strings[0].to_string(),
            n,
            Some(m),
        ))),
        (Err(_), _) => Err(format!("invalid offset: {}", input_strings[1])),
        (_, Err(_)) => Err(format!("invalid label: {}", input_strings[2])),
    }
}

/// 解析命令行中用于偏移量和标签的格式
pub fn parse_offset_operand(s: &str) -> Result<u64, &'static str> {
    let mut start = 0;
    let mut len = s.len();
    let mut radix = 8; // 默认使用8进制
    let mut multiply = 1; // 乘数因子

    // 处理可选的 '+' 前缀
    if s.starts_with('+') {
        start += 1;
    }

    // 处理十六进制前缀
    if s[start..len].starts_with("0x") || s[start..len].starts_with("0X") {
        start += 2;
        radix = 16;
    } else {
        // 处理后缀
        if s[start..len].ends_with('b') {
            len -= 1;
            multiply = 512; // 'b' 后缀表示乘以512
        }
        if s[start..len].ends_with('.') {
            len -= 1;
            radix = 10; // '.' 后缀表示使用十进制
        }
    }

    // 解析数值
    match u64::from_str_radix(&s[start..len], radix) {
        Ok(i) => Ok(i * multiply),
        Err(_) => Err("解析失败"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ct_app;

    #[test]
    fn test_parse_inputs_normal() {
        assert_eq!(
            CommandLineInputs::FileNames(vec!["-".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od"])).unwrap()
        );

        assert_eq!(
            CommandLineInputs::FileNames(vec!["-".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "-"])).unwrap()
        );

        assert_eq!(
            CommandLineInputs::FileNames(vec!["file1".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "file1"])).unwrap()
        );

        assert_eq!(
            CommandLineInputs::FileNames(vec!["file1".to_string(), "file2".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "file1", "file2"])).unwrap()
        );

        assert_eq!(
            CommandLineInputs::FileNames(vec![
                "-".to_string(),
                "file1".to_string(),
                "file2".to_string(),
            ]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "-", "file1", "file2"])).unwrap()
        );
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_parse_inputs_with_offset() {
        // offset is found without filename, so stdin will be used.
        assert_eq!(
            CommandLineInputs::FileAndOffset(("-".to_string(), 8, None)),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "+10"])).unwrap()
        );

        // offset must start with "+" if no input is specified.
        assert_eq!(
            CommandLineInputs::FileNames(vec!["10".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "10"])).unwrap()
        );

        // offset is not valid, so it is considered a filename.
        assert_eq!(
            CommandLineInputs::FileNames(vec!["+10a".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "+10a"])).unwrap()
        );

        // if -j is included in the command line, there cannot be an offset.
        assert_eq!(
            CommandLineInputs::FileNames(vec!["+10".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "-j10", "+10"])).unwrap()
        );

        // if -v is included in the command line, there cannot be an offset.
        assert_eq!(
            CommandLineInputs::FileNames(vec!["+10".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "-o", "-v", "+10"])).unwrap()
        );

        assert_eq!(
            CommandLineInputs::FileAndOffset(("file1".to_string(), 8, None)),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "file1", "+10"])).unwrap()
        );

        // offset does not need to start with "+" if a filename is included.
        assert_eq!(
            CommandLineInputs::FileAndOffset(("file1".to_string(), 8, None)),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "file1", "10"])).unwrap()
        );

        assert_eq!(
            CommandLineInputs::FileNames(vec!["file1".to_string(), "+10a".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "file1", "+10a"])).unwrap()
        );

        assert_eq!(
            CommandLineInputs::FileNames(vec!["file1".to_string(), "+10".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "-j10", "file1", "+10"]))
                .unwrap()
        );

        // offset must be last on the command line
        assert_eq!(
            CommandLineInputs::FileNames(vec!["+10".to_string(), "file1".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "+10", "file1"])).unwrap()
        );
    }

    #[test]
    fn test_parse_inputs_traditional() {
        // it should not return FileAndOffset to signal no offset was entered on the command line.
        assert_eq!(
            CommandLineInputs::FileNames(vec!["-".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "--traditional"])).unwrap()
        );

        assert_eq!(
            CommandLineInputs::FileNames(vec!["file1".to_string()]),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "--traditional", "file1"]))
                .unwrap()
        );

        // offset does not need to start with a +
        assert_eq!(
            CommandLineInputs::FileAndOffset(("-".to_string(), 8, None)),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "--traditional", "10"])).unwrap()
        );

        // valid offset and valid label
        assert_eq!(
            CommandLineInputs::FileAndOffset(("-".to_string(), 8, Some(8))),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "--traditional", "10", "10"]))
                .unwrap()
        );

        assert_eq!(
            CommandLineInputs::FileAndOffset(("file1".to_string(), 8, None)),
            od_parse_inputs(&ct_app().get_matches_from(vec!["od", "--traditional", "file1", "10"]))
                .unwrap()
        );

        // only one file is allowed, it must be the first
        od_parse_inputs(&ct_app().get_matches_from(vec!["od", "--traditional", "10", "file1"]))
            .unwrap_err();

        assert_eq!(
            CommandLineInputs::FileAndOffset(("file1".to_string(), 8, Some(8))),
            od_parse_inputs(&ct_app().get_matches_from(vec![
                "od",
                "--traditional",
                "file1",
                "10",
                "10"
            ]))
            .unwrap()
        );

        od_parse_inputs(&ct_app().get_matches_from(vec![
            "od",
            "--traditional",
            "10",
            "file1",
            "10",
        ]))
        .unwrap_err();

        od_parse_inputs(&ct_app().get_matches_from(vec![
            "od",
            "--traditional",
            "10",
            "10",
            "file1",
        ]))
        .unwrap_err();

        od_parse_inputs(&ct_app().get_matches_from(vec![
            "od",
            "--traditional",
            "10",
            "10",
            "10",
            "10",
        ]))
        .unwrap_err();
    }

    fn parse_offset_operand_str(s: &str) -> Result<u64, &'static str> {
        parse_offset_operand(&String::from(s))
    }

    #[test]
    fn test_parse_offset_operand_invalid() {
        parse_offset_operand_str("").unwrap_err();
        parse_offset_operand_str("a").unwrap_err();
        parse_offset_operand_str("+").unwrap_err();
        parse_offset_operand_str("+b").unwrap_err();
        parse_offset_operand_str("0x1.").unwrap_err();
        parse_offset_operand_str("0x1.b").unwrap_err();
        parse_offset_operand_str("-").unwrap_err();
        parse_offset_operand_str("-1").unwrap_err();
        parse_offset_operand_str("1e10").unwrap_err();
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_parse_offset_operand() {
        assert_eq!(8, parse_offset_operand_str("10").unwrap()); // default octal
        assert_eq!(0, parse_offset_operand_str("0").unwrap());
        assert_eq!(8, parse_offset_operand_str("+10").unwrap()); // optional leading '+'
        assert_eq!(16, parse_offset_operand_str("0x10").unwrap()); // hex
        assert_eq!(16, parse_offset_operand_str("0X10").unwrap()); // hex
        assert_eq!(16, parse_offset_operand_str("+0X10").unwrap()); // hex
        assert_eq!(10, parse_offset_operand_str("10.").unwrap()); // decimal
        assert_eq!(10, parse_offset_operand_str("+10.").unwrap()); // decimal
        assert_eq!(4096, parse_offset_operand_str("10b").unwrap()); // b suffix = *512
        assert_eq!(4096, parse_offset_operand_str("+10b").unwrap()); // b suffix = *512
        assert_eq!(5120, parse_offset_operand_str("10.b").unwrap()); // b suffix = *512
        assert_eq!(5120, parse_offset_operand_str("+10.b").unwrap()); // b suffix = *512
        assert_eq!(267, parse_offset_operand_str("0x10b").unwrap()); // hex
    }

    #[test]
    fn test_od_parse_inputs_basic() {
        // 测试基本的输入解析
        let mut mock_opts = MockCommandLineOpts::new();

        // 测试空输入（应该返回标准输入）
        mock_opts.set_inputs(vec![]);
        let result = od_parse_inputs(&mock_opts).unwrap();
        assert_eq!(result, CommandLineInputs::FileNames(vec!["-".to_string()]));

        // 测试单个文件
        mock_opts.set_inputs(vec!["file1"]);
        let result = od_parse_inputs(&mock_opts).unwrap();
        assert_eq!(
            result,
            CommandLineInputs::FileNames(vec!["file1".to_string()])
        );

        // 测试多个文件
        mock_opts.set_inputs(vec!["file1", "file2"]);
        let result = od_parse_inputs(&mock_opts).unwrap();
        assert_eq!(
            result,
            CommandLineInputs::FileNames(vec!["file1".to_string(), "file2".to_string()])
        );
    }

    #[test]
    fn test_od_parse_inputs_with_offset() {
        let mut mock_opts = MockCommandLineOpts::new();

        // 测试带偏移量的标准输入
        mock_opts.set_inputs(vec!["+10"]);
        let result = od_parse_inputs(&mock_opts).unwrap();
        assert_eq!(
            result,
            CommandLineInputs::FileAndOffset(("-".to_string(), 8, None))
        );

        // 测试带偏移量的文件
        mock_opts.set_inputs(vec!["file1", "+10"]);
        let result = od_parse_inputs(&mock_opts).unwrap();
        assert_eq!(
            result,
            CommandLineInputs::FileAndOffset(("file1".to_string(), 8, None))
        );

        // 测试无效偏移量
        mock_opts.set_inputs(vec!["file1", "+xyz"]);
        let result = od_parse_inputs(&mock_opts).unwrap();
        assert_eq!(
            result,
            CommandLineInputs::FileNames(vec!["file1".to_string(), "+xyz".to_string()])
        );
    }

    #[test]
    fn test_od_parse_inputs_traditional() {
        // 测试空输入
        let result = od_parse_inputs_traditional(&[]).unwrap();
        assert_eq!(result, CommandLineInputs::FileNames(vec!["-".to_string()]));

        // 测试单个文件
        let result = od_parse_inputs_traditional(&["file1"]).unwrap();
        assert_eq!(
            result,
            CommandLineInputs::FileNames(vec!["file1".to_string()])
        );

        // 测试偏移量
        let result = od_parse_inputs_traditional(&["10"]).unwrap();
        assert_eq!(
            result,
            CommandLineInputs::FileAndOffset(("-".to_string(), 8, None))
        );

        // 测试文件和偏移量
        let result = od_parse_inputs_traditional(&["file1", "10"]).unwrap();
        assert_eq!(
            result,
            CommandLineInputs::FileAndOffset(("file1".to_string(), 8, None))
        );

        // 测试带标签的偏移量
        let result = od_parse_inputs_traditional(&["file1", "10", "20"]).unwrap();
        assert_eq!(
            result,
            CommandLineInputs::FileAndOffset(("file1".to_string(), 8, Some(16)))
        );
    }

    #[test]
    fn test_od_parse_inputs_traditional_errors() {
        // 测试无效偏移量
        assert!(od_parse_inputs_traditional(&["file1", "xyz"]).is_err());

        // 测试无效标签
        assert!(od_parse_inputs_traditional(&["file1", "10", "xyz"]).is_err());

        // 测试参数过多
        assert!(od_parse_inputs_traditional(&["10", "20", "30", "40"]).is_err());

        // 测试偏移量位置错误
        assert!(od_parse_inputs_traditional(&["10", "file1"]).is_err());
    }

    // 用于测试的 Mock 实现
    struct MockCommandLineOpts {
        inputs: Vec<&'static str>,
        opts: Vec<&'static str>,
    }

    impl MockCommandLineOpts {
        fn new() -> Self {
            Self {
                inputs: Vec::new(),
                opts: Vec::new(),
            }
        }

        fn set_inputs(&mut self, inputs: Vec<&'static str>) {
            self.inputs = inputs;
        }
    }

    impl CommandLineOpts for MockCommandLineOpts {
        fn inputs(&self) -> Vec<&str> {
            self.inputs.clone()
        }

        fn opts_present(&self, opts: &[&str]) -> bool {
            opts.iter().any(|opt| self.opts.contains(opt))
        }
    }
}
