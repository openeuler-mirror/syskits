/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

use clap::crate_version;
use clap::Arg;
use clap::ArgAction;
use clap::Command;

use ctcore::ct_error::CTResult;
use ctcore::ct_error::FromIo;

use ctcore::ct_format_usage;
use ctcore::ct_help_about;
use ctcore::ct_help_section;
use ctcore::ct_help_usage;

use std::io;
use std::io::Write;
use std::iter::Peekable;
use std::ops::ControlFlow;
use std::str::Chars;

const ECHO_ABOUT: &str = ct_help_about!("echo.md");
const ECHO_USAGE: &str = ct_help_usage!("echo.md");
const ECHO_AFTER_HELP: &str = ct_help_section!("after help", "echo.md");

mod opt_flags {
    pub const STRING: &str = "STRING";
    pub const NO_NEWLINE: &str = "no_newline";
    pub const ENABLE_BACKSLASH_ESCAPE: &str = "enable_backslash_escape";
    pub const DISABLE_BACKSLASH_ESCAPE: &str = "disable_backslash_escape";
}

#[repr(u8)]
// 定义支持的基数枚举，及其最大数字位数
#[derive(Clone, Copy)]
enum EchoBase {
    Oct = 8,  // 八进制
    Hex = 16, // 十六进制
}

impl EchoBase {
    /// 返回基数的最大数字位数
    fn max_digits(&self) -> u8 {
        match self {
            Self::Oct => 3,
            Self::Hex => 2,
        }
    }
}

/// 解析`\xHHH`和`\0NNN`转义序列中的数值部分
fn echo_parse_code(input: &mut Peekable<Chars>, base: EchoBase) -> Option<char> {
    // 由于八进制输入可能需要3个数字，这超过了`u8`的容量，因此这里需要使用溢出加法。
    // 注意，如果使用`u32`和`char::from_u32`，则会对大于`u8::MAX`的值错误地解释为Unicode字符。
    let mut ret = input.peek().and_then(|c| c.to_digit(base as u32))? as u8;

    // 安全地忽略`None`情况，因为我们只是进行了预览。
    let _ = input.next();

    // 处理剩余的数字字符，根据基数进行解析
    for _ in 1..base.max_digits() {
        match input.peek().and_then(|c| c.to_digit(base as u32)) {
            Some(n) => ret = ret.wrapping_mul(base as u8).wrapping_add(n as u8),
            None => break,
        }
        // 安全地忽略`None`情况，因为我们只是进行了预览。
        let _ = input.next();
    }

    Some(ret.into())
}

/// 将转义序列写入给定的输出流
fn echo_print_escaped(input: &str, mut output: impl Write) -> io::Result<ControlFlow<()>> {
    let mut iter = input.chars().peekable();
    while let Some(c) = iter.next() {
        if c != '\\' {
            write!(output, "{c}")?;
            continue;
        }

        // 处理八进制转义序列（\NNN）的逻辑
        if let Some('1'..='8') = iter.peek() {
            if let Some(parsed) = echo_parse_code(&mut iter, EchoBase::Oct) {
                write!(output, "{parsed}")?;
                continue;
            }
        }

        if let Some(next) = iter.next() {
            let unescaped = match next {
                '\\' => '\\',
                'a' => '\x07',
                'b' => '\x08',
                'c' => return Ok(ControlFlow::Break(())),
                'e' => '\x1b',
                'f' => '\x0c',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'v' => '\x0b',
                'x' => {
                    if let Some(c) = echo_parse_code(&mut iter, EchoBase::Hex) {
                        c
                    } else {
                        write!(output, "\\")?;
                        'x'
                    }
                }
                '0' => echo_parse_code(&mut iter, EchoBase::Oct).unwrap_or('\0'),
                c => {
                    write!(output, "\\")?;
                    c
                }
            };
            write!(output, "{unescaped}")?;
        } else {
            write!(output, "\\")?;
        }
    }

    Ok(ControlFlow::Continue(()))
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    echo_main(args).map(|_| ())
}

pub fn echo_main(args: impl ctcore::Args) -> CTResult<()> {
    let args_match = ct_app()
        .after_help(ECHO_AFTER_HELP)
        .try_get_matches_from(args)?;

    let no_newline = args_match.get_flag(opt_flags::NO_NEWLINE);
    let escaped = args_match.get_flag(opt_flags::ENABLE_BACKSLASH_ESCAPE);
    let values: Vec<String> = match args_match.get_many::<String>(opt_flags::STRING) {
        Some(s) => s.map(|s| s.to_string()).collect(),
        None => vec![String::new()],
    };

    echo_execute(no_newline, escaped, &values)
        .map_err_context(|| "could not write to stdout".to_string())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = ECHO_ABOUT;
    let usage_description = ct_format_usage(ECHO_USAGE);

    let args = vec![
        Arg::new(opt_flags::NO_NEWLINE)
            .short('n')
            .help("do not output the trailing newline")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::ENABLE_BACKSLASH_ESCAPE)
            .short('e')
            .help("enable interpretation of backslash escapes")
            .action(ArgAction::SetTrue)
            .overrides_with(opt_flags::DISABLE_BACKSLASH_ESCAPE),
        Arg::new(opt_flags::DISABLE_BACKSLASH_ESCAPE)
            .short('E')
            .help("disable interpretation of backslash escapes (default)")
            .action(ArgAction::SetTrue)
            .overrides_with(opt_flags::ENABLE_BACKSLASH_ESCAPE),
        Arg::new(opt_flags::STRING).action(ArgAction::Append),
    ];

    Command::new(utility_name)
        // TrailingVarArg指定最后一个位置参数是一个VarArg，并且它不会进一步尝试解析任何其他参数。
        .trailing_var_arg(true)
        .allow_hyphen_values(true)
        .version(command_version)
        .about(application_info)
        .after_help(ECHO_AFTER_HELP)
        .override_usage(usage_description)
        .args(&args)
}

fn echo_execute(no_newline: bool, escaped: bool, free: &[String]) -> io::Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();

    for (i, input) in free.iter().enumerate() {
        if i > 0 {
            write!(output, " ")?;
        }
        if escaped {
            // 如果处理转义序列，使用`echo_print_escaped`函数
            if echo_print_escaped(input, &mut output)?.is_break() {
                return Ok(());
            }
        } else {
            // 如果不处理转义序列，直接写入
            write!(output, "{input}")?;
        }
    }

    // 如果未指定不输出换行符，则输出换行符
    if !no_newline {
        writeln!(output)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {

    mod tests_echo_main {
        use crate::echo_main;

        use std::fs;
        use std::fs::File;
        use tempfile::Builder;

        use std::ffi::OsString;
        use std::io::Write;
        #[test]
        fn test_echo_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = echo_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_echo_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = echo_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_echo_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = echo_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_echo_main_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = echo_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_echo_main_n() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), "-n", "12345", ">", filename1];
            let result = echo_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_main_e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), "-e", "12345", ">", filename1];
            let result = echo_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_main_ee() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), "-E", "12345", ">", filename1];
            let result = echo_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }

    mod tests_ct_app {
        use crate::ct_app;

        use clap::error::ErrorKind;

        #[test]
        fn test_echo_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_echo_app_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_echo_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_echo_app_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_echo_app_n() {
            let args = vec![ctcore::ct_util_name(), "-n"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_app_e() {
            let args = vec![ctcore::ct_util_name(), "-e"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_app_ee() {
            let args = vec![ctcore::ct_util_name(), "-E"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
    }
}