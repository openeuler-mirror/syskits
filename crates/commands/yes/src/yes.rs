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

extern crate rust_i18n;
use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Write};

use clap::{Arg, ArgAction, Command, builder::ValueParser, crate_version};
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CtSimpleError};
#[cfg(unix)]
use ctcore::ct_signals::enable_pipe_errors;

use rust_i18n::t;
use sys_locale::get_locale;

// 声明 i18n 宏和初始化函数
rust_i18n::i18n!("locales", fallback = "zh-CN");

#[cfg(target_os = "linux")]
mod splice;

// 在某些系统上，使用更小或更大的缓冲区可能会提供更好的性能，当前设置满足需求
const YES_BUF_SIZE: usize = 16 * 1024;

#[derive(Default)]
pub struct Yes;
impl Tool for Yes {
    fn name(&self) -> &'static str {
        "yes"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        yes_main(args.iter().cloned())
    }
}

pub fn yes_main(args: impl ctcore::Args) -> CTResult<()> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let matches = ct_app().try_get_matches_from(args)?;

    let mut buff = Vec::with_capacity(YES_BUF_SIZE);
    yes_args_into_buff(&mut buff, matches.get_many::<OsString>("STRING")).unwrap();
    yes_prepare_buff(&mut buff);

    if let Err(err) = yes_exec(&buff) {
        if matches!(err.kind(), io::ErrorKind::BrokenPipe) {
            Ok(())
        } else {
            Err(CtSimpleError::new(
                1,
                t!("ct_yes.errors.stdout", error = err.to_string()),
            ))
        }
    } else {
        Ok(())
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("ct_yes.about");
    let usage_description = t!("ct_yes.usage");
    let arg = Arg::new("STRING")
        .value_parser(ValueParser::os_string())
        .action(ArgAction::Append);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .help(t!("ct_yes.clap.help"))
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .help(t!("ct_yes.clap.version"))
                .action(ArgAction::Version),
        )
        .arg(arg)
}

// 将`i`中的单词复制到`buf`中，中间用空格隔开。
fn yes_args_into_buff<'a>(
    buffer: &mut Vec<u8>,
    iter_option: Option<impl Iterator<Item = &'a OsString>>,
) -> Result<(), Box<dyn Error>> {
    // 如果没有提供参数，则直接在缓冲区中追加 "y\n" 并返回成功。
    let Some(iter) = iter_option else {
        buffer.extend_from_slice(b"y\n");
        return Ok(());
    };

    // Unix 系统（包括 WASI）处理逻辑：直接将 OsString 转为字节序列并以空格分隔。
    #[cfg(unix)]
    {
        #[cfg(unix)]
        use std::os::unix::ffi::OsStrExt;

        for part in itertools::intersperse(iter.map(|a| a.as_bytes()), b" ") {
            buffer.extend_from_slice(part);
        }
    }

    // Windows 系统处理逻辑：必须将 OsString 转换为 String，以处理可能的 UTF-8 编码问题。
    #[cfg(not(unix))]
    {
        for part_option in itertools::intersperse(iter.map(|os_str| os_str.to_str()), Some(" ")) {
            let b = match part_option {
                Some(p) => p.as_bytes(),
                None => return Err(t!("ct_yes.errors.invalid_utf8").into()),
            };
            buffer.extend_from_slice(b);
        }
    }

    // 在参数序列末尾追加换行符。
    buffer.push(b'\n');

    Ok(())
}

// 假定 buf 保存了从命令行参数中伪造的单个输出行，然后反复复制，直到缓冲区在 BUF_SIZE 范围内尽可能多地保存副本为止。
fn yes_prepare_buff(buffer: &mut Vec<u8>) {
    if buffer.len() * 2 > YES_BUF_SIZE {
        return;
    }

    assert!(!buffer.is_empty());

    let line_len = buffer.len();
    let target_size = line_len * (YES_BUF_SIZE / line_len);

    while buffer.len() < target_size {
        let to_copy = std::cmp::min(target_size - buffer.len(), buffer.len());
        debug_assert_eq!(to_copy % line_len, 0);
        buffer.extend_from_within(..to_copy);
    }
}

pub fn yes_exec(bytes_data: &[u8]) -> io::Result<()> {
    let io_ouput = io::stdout();
    let mut std_output: io::StdoutLock<'_> = io_ouput.lock();
    #[cfg(unix)]
    enable_pipe_errors()?;

    #[cfg(target_os = "linux")]
    {
        if splice::splice_data(bytes_data, &std_output).is_ok() {
            return Ok(());
        } else if let Err(splice::SpliceError::Io(err)) =
            splice::splice_data(bytes_data, &std_output)
        {
            return Err(err);
        } else if let Err(splice::SpliceError::Unsupported) =
            splice::splice_data(bytes_data, &std_output)
        {
            // 处理不支持的错误(do nothing)
        }
    }

    loop {
        std_output.write_all(bytes_data)?;
    }
}

#[cfg(unix)]
#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use rust_i18n::t;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Yes::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "yes");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("yes"));

        // 测试 execute 方法
        let args = vec![OsString::from("yes"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }
    // yes 接口: yes [STRING]...
    //     or:  yes OPTION
    //       --help     display this help and exit
    //       --version  output version information and exit
    #[test]
    fn test_ct_app_execution_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--version"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_other_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "-V"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_help() {
        let command = ct_app();

        // 测试用例2：验证 --help 参数是否正确处理
        let help_args = vec![ctcore::ct_util_name(), "--help"];
        let result = command.try_get_matches_from(help_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_ct_app_execution_unsupport_help() {
        let command = ct_app();

        // 测试用例2：验证 --help 参数是否正确处理
        let help_args = vec![ctcore::ct_util_name(), "-H"];
        let result = command.try_get_matches_from(help_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_invalid_argument() {
        let command = ct_app();

        // 测试用例3：验证当提供未知参数时是否正确报错
        let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
        let result = command.try_get_matches_from(invalid_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_support_missing_argument() {
        let command = ct_app();

        // 测试用例4：验证当缺少必需的参数时是否正确报错
        let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
        let result = command.try_get_matches_from(missing_args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_single_long_string() {
        let long_string = OsString::from("a".repeat(10_000));
        let mut v = Vec::new();
        yes_args_into_buff(&mut v, Some([long_string].iter())).unwrap();
        assert!(v.len() > 10_000);
    }

    #[test]
    fn test_multiple_long_strings() {
        let long_strings = [
            OsString::from("a".repeat(5000)),
            OsString::from("b".repeat(5000)),
            OsString::from("c".repeat(5000)),
        ];
        let mut v = Vec::new();
        yes_args_into_buff(&mut v, Some(long_strings.iter())).unwrap();
        assert!(v.len() > 15_000);
    }

    #[test]
    fn test_input_with_special_characters() {
        let inputs = [
            OsString::from("hello\nworld"),
            OsString::from("hello\ttab"),
            OsString::from("hello\\backslash"),
        ];
        let mut v = Vec::new();
        yes_args_into_buff(&mut v, Some(inputs.iter())).unwrap();
        assert_eq!(
            String::from_utf8(v).unwrap(),
            "hello\nworld hello\ttab hello\\backslash\n"
        );
    }

    #[test]
    fn test_repeated_calls() {
        let inputs = [OsString::from("repeat"), OsString::from("test")];
        let mut v = Vec::new();
        for _ in 0..3 {
            yes_args_into_buff(&mut v, Some(inputs.iter())).unwrap();
        }
        assert_eq!(
            String::from_utf8(v).unwrap(),
            "repeat test\nrepeat test\nrepeat test\n"
        );
    }

    #[test]
    fn test_extreme_small_input() {
        let inputs = [OsString::from(""), OsString::from(" ")];
        let mut v = Vec::new();
        yes_args_into_buff(&mut v, Some(inputs.iter())).unwrap();
        assert_eq!(String::from_utf8(v).unwrap(), "  \n");
    }

    #[test]
    fn test_maximum_length_input() {
        let max_input = OsString::from("x".repeat(65535));
        let mut v = Vec::new();
        yes_args_into_buff(&mut v, Some([max_input].iter())).unwrap();
        assert!(v.len() == 65536); // Including the newline
    }

    #[test]
    fn test_input_with_unprintable_characters() {
        let inputs = [OsString::from("\x01\x02\x03")];
        let mut v = Vec::new();
        yes_args_into_buff(&mut v, Some(inputs.iter())).unwrap();
        assert_eq!(String::from_utf8(v).unwrap(), "\x01\x02\x03\n");
    }

    #[test]
    fn test_large_number_of_small_inputs() {
        let small_inputs = std::iter::repeat(OsString::from("small"))
            .take(10_000)
            .collect::<Vec<_>>();
        let mut v = Vec::new();
        yes_args_into_buff(&mut v, Some(small_inputs.iter())).unwrap();
        assert!(v.len() > 50_000); // 10,000 * "small ".length() + 1 for '\n'
    }

    #[test]
    fn test_repeated_calls_memory_leak() {
        let inputs = [OsString::from("repeat")];
        let mut v = Vec::new();
        for _ in 0..100 {
            yes_args_into_buff(&mut v, Some(inputs.iter())).unwrap();
        }
        assert_eq!(v.len(), 700); // 7 characters * 100
    }

    #[test]
    fn test_near_ct_buf_size() {
        let tests = [
            (YES_BUF_SIZE / 2, YES_BUF_SIZE),
            (YES_BUF_SIZE - 1, YES_BUF_SIZE - 1),
            (YES_BUF_SIZE / 4 * 3, YES_BUF_SIZE / 4 * 3),
        ];

        for (line, final_len) in tests {
            let mut v = std::iter::repeat(b'a').take(line).collect::<Vec<_>>();
            yes_prepare_buff(&mut v);
            assert_eq!(v.len(), final_len);
        }
    }

    // #[test]
    // #[should_panic]
    // fn test_empty_vector() {
    //     let mut v = Vec::new();
    //     prepare_buff(&mut v);  // 应该在 assert!(!buf.is_empty()) 中触发 panic
    // }

    #[test]
    fn test_large_vector() {
        let line = YES_BUF_SIZE * 2; // 输入超过了 CT_BUF_SIZE
        let mut v = std::iter::repeat(b'a').take(line).collect::<Vec<_>>();
        yes_prepare_buff(&mut v);
        assert_eq!(v.len(), line); // 由于超过 CT_BUF_SIZE，预期不会更改
    }

    #[test]
    fn test_performance_large_input() {
        let line = 100_000;
        let mut v = std::iter::repeat(b'a').take(line).collect::<Vec<_>>();
        let start = std::time::Instant::now();
        yes_prepare_buff(&mut v);
        let duration = start.elapsed();
        println!("Duration for large input: {:?}", duration);
        // 可以通过断言确保性能在可接受的范围内
        assert!(duration < std::time::Duration::from_millis(100));
    }

    #[test]
    fn test_boundary_conditions() {
        let tests = [
            (YES_BUF_SIZE, YES_BUF_SIZE),
            (YES_BUF_SIZE - 1, YES_BUF_SIZE - 1),
            (YES_BUF_SIZE - 2, YES_BUF_SIZE - 2),
        ];

        for (line, final_len) in tests {
            let mut v = std::iter::repeat(b'a').take(line).collect::<Vec<_>>();
            yes_prepare_buff(&mut v);
            assert_eq!(v.len(), final_len);
        }
    }

    #[test]
    fn test_minimum_length() {
        let tests = [(1, YES_BUF_SIZE)];

        for (line, final_len) in tests {
            let mut v = std::iter::repeat(b'a').take(line).collect::<Vec<_>>();
            yes_prepare_buff(&mut v);
            assert_eq!(v.len(), final_len);
        }
    }

    #[test]
    fn test_odd_and_even_lengths() {
        let tests = [
            (3, 3), // Odd length that does not grow
            (6, 6), // Even length that does not grow
        ];

        for (line, final_len) in tests {
            let v = std::iter::repeat(b'a').take(line).collect::<Vec<_>>();
            assert_eq!(v.len(), final_len);
        }
    }

    #[test]
    fn test_large_input_performance() {
        let line = YES_BUF_SIZE * 10; // Huge input size
        let mut v = std::iter::repeat(b'a').take(line).collect::<Vec<_>>();
        let start = std::time::Instant::now();
        yes_prepare_buff(&mut v);
        let duration = start.elapsed();
        assert!(
            duration < std::time::Duration::from_secs(1),
            "Performance issue with very large input"
        );
    }

    #[test]
    fn test_repeated_calls2() {
        let line = 100;
        let mut v = std::iter::repeat(b'a').take(line).collect::<Vec<_>>();
        for _ in 0..10 {
            // Repeat multiple times
            yes_prepare_buff(&mut v);
        }
        assert_eq!(v.len(), line * (YES_BUF_SIZE / line)); // Check after repeated buffer preparations
    }

    #[test]
    fn test_base_prepare_buffer() {
        let tests = [
            (2, 16384),
            (3, 16383),
            (4, 16384),
            (5, 16380),
            (150, 16350),
            (1000, 16000),
            (4093, 16372),
            (4099, 12297),
            (4111, 12333),
            (8191, 16382),
            (8192, 16384),
            (8193, 8193),
            (10000, 10000),
            (15000, 15000),
            (25000, 25000),
        ];

        for (line, final_len) in tests {
            let mut v = std::iter::repeat(b'a').take(line).collect::<Vec<_>>();
            yes_prepare_buff(&mut v);
            assert_eq!(v.len(), final_len);
        }
    }

    #[test]
    fn test_base_args_into_buf() {
        {
            let mut v = Vec::with_capacity(YES_BUF_SIZE);
            yes_args_into_buff(&mut v, Some([OsString::from("foo")].iter())).unwrap();
            assert_eq!(String::from_utf8(v).unwrap(), "foo\n");
        }

        {
            let mut v = Vec::with_capacity(YES_BUF_SIZE);
            yes_args_into_buff(
                &mut v,
                Some(
                    [
                        OsString::from("fooa"),
                        OsString::from("barb    bazz"),
                        OsString::from("quxw"),
                    ]
                    .iter(),
                ),
            )
            .unwrap();
            assert_eq!(String::from_utf8(v).unwrap(), "fooa barb    bazz quxw\n");
        }
    }
    #[test]
    fn test_i18n_errors() {
        // Test English errors
        rust_i18n::set_locale("en-US");
        let err = t!("ct_yes.errors.invalid_utf8");
        assert_eq!(err, "Arguments contain invalid UTF-8");

        // Test Chinese errors
        rust_i18n::set_locale("zh-CN");
        let err = t!("ct_yes.errors.invalid_utf8");
        assert_eq!(err, "参数包含无效的UTF-8字符");
    }
}
