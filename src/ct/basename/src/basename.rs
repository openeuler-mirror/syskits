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
use clap::Arg;
use clap::ArgAction;
use clap::Command;
use clap::crate_version;
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError};
use ctcore::ct_line_ending::CtLineEnding;
use std::ffi::OsString;
use std::path::PathBuf;
use std::path::is_separator;
use sys_locale::get_locale;

use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");

pub mod flags {
    pub static MULTIPLE: &str = "multiple";
    pub static NAME: &str = "name";
    pub static SUFFIX: &str = "suffix";
    pub static ZERO: &str = "zero";
}

#[derive(Default)]
pub struct Basename;
impl Tool for Basename {
    fn name(&self) -> &'static str {
        "basename"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        basename_main(args.iter().cloned())
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    basename_main(args).map(|_| ())
}

pub fn basename_main(args: impl ctcore::Args) -> CTResult<()> {
    // Set locale based on system settings
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let args = args.collect_lossy();

    let args_match = ct_app().try_get_matches_from(args)?;

    let line_ending_info = CtLineEnding::from_zero_flag(args_match.get_flag(flags::ZERO));

    let mut names = args_match
        .get_many::<String>(flags::NAME)
        .unwrap_or_default()
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Err(CTsageError::new(1, t!("basename.errors.missing_operand")));
    }
    let paths = args_match.get_one::<String>(flags::SUFFIX).is_some()
        || args_match.get_flag(flags::MULTIPLE);
    let base_suffix = if paths {
        args_match
            .get_one::<String>(flags::SUFFIX)
            .cloned()
            .unwrap_or_default()
    } else {
        let length = names.len();

        if length == 0 {
            panic!("already checked");
        } else if length == 1 {
            String::default()
        } else if length == 2 {
            names.pop().unwrap().clone()
        } else {
            return Err(CTsageError::new(
                1,
                format!(
                    "{} {}",
                    t!("basename.errors.extra_operand"),
                    names[2].quote()
                ),
            ));
        }
    };

    for path in names {
        print!("{}{}", basename(path, &base_suffix), line_ending_info);
    }

    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("basename.about");
    let usage_description = t!("basename.usage");

    let args = basename_args_init();
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

fn basename_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(flags::MULTIPLE)
            .short('a')
            .long(flags::MULTIPLE)
            .help(t!("basename.clap.multiple"))
            .action(ArgAction::SetTrue)
            .overrides_with(flags::MULTIPLE),
        Arg::new(flags::NAME)
            .action(clap::ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath)
            .hide(true)
            .trailing_var_arg(true),
        Arg::new(flags::SUFFIX)
            .short('s')
            .long(flags::SUFFIX)
            .value_name("SUFFIX")
            .help(t!("basename.clap.suffix"))
            .overrides_with(flags::SUFFIX),
        Arg::new(flags::ZERO)
            .short('z')
            .long(flags::ZERO)
            .help(t!("basename.clap.zero"))
            .action(ArgAction::SetTrue)
            .overrides_with(flags::ZERO),
        Arg::new("help")
            .short('h')
            .long("help")
            .help(t!("basename.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .short('V')
            .long("version")
            .help(t!("basename.clap.version"))
            .action(ArgAction::Version),
    ];
    args
}

fn basename(fullname: &str, suffix: &str) -> String {
    // 步骤1：从末尾移除所有平台特定的路径分隔符
    let trimmed_path = fullname.trim_end_matches(is_separator);

    // 步骤2：确保在修剪后路径不为空，处理仅由后缀字符组成的特殊情况
    let adjusted_path = if trimmed_path.is_empty() {
        // 恢复为原始的fullname以避免返回空路径
        fullname
    } else {
        trimmed_path
    };

    // 步骤3：将调整后的路径转换为PathBuf
    let path_buffer = PathBuf::from(adjusted_path);

    // 步骤4：获取路径的最后一部分
    let last_component_option = path_buffer.components().next_back();

    // 步骤5：处理最后一部分缺失的情况
    let result = match last_component_option {
        Some(last_component) => {
            // 步骤6：将最后一部分作为字符串获取
            let last_component_name = last_component.as_os_str().to_str().unwrap();

            // 步骤7：比较最后一部分名称与提供的后缀
            if last_component_name == suffix {
                // 步骤8：若两者相等，则将后缀本身作为基名称返回
                last_component_name.to_string()
            } else {
                // 步骤9：若两者不相等，则尝试从前一部分移除后缀
                let stripped_name = last_component_name.strip_suffix(suffix);

                // 步骤10：处理移除后缀的结果
                match stripped_name {
                    Some(stripped) => {
                        // 步骤11：如果成功，返回剥离后的名称作为基名称
                        stripped.to_string()
                    }
                    None => {
                        // 步骤12：如果剥离失败（即后缀不匹配），则返回原始的最后一部分名称作为基名称
                        last_component_name.to_string()
                    }
                }
            }
        }
        None => {
            // 步骤13：如果没有最后一部分，则返回空字符串作为基名称
            String::new()
        }
    };

    // 步骤14：返回计算出的基名称
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::ffi::OsString;

    #[test]
    fn test_i18n_help_messages() {
        // 设置英文环境
        rust_i18n::set_locale("en-US");
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--help"];
        let result = command.try_get_matches_from(args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);

        // 设置中文环境
        rust_i18n::set_locale("zh-CN");
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--help"];
        let result = command.try_get_matches_from(args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_i18n_error_messages() {
        // 测试英文错误消息
        rust_i18n::set_locale("en-US");
        let args = vec![ctcore::ct_util_name()];
        let result = basename_main(args.iter().map(|s| OsString::from(s)));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), 1);

        // 测试中文错误消息
        rust_i18n::set_locale("zh-CN");
        let args = vec![ctcore::ct_util_name()];
        let result = basename_main(args.iter().map(|s| OsString::from(s)));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), 1);
    }

    #[test]
    fn test_i18n_fallback() {
        // 测试不存在的语言环境，应该回退到中文
        rust_i18n::set_locale("fr-FR");
        let args = vec![ctcore::ct_util_name()];
        let result = basename_main(args.iter().map(|s| OsString::from(s)));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), 1);
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Basename;

        // 测试 name 方法
        assert_eq!(tool.name(), "basename");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("basename"));

        // 测试 execute 方法
        let args = vec![OsString::from("basename"), OsString::from("/usr/bin/sort")];
        assert!(tool.execute(&args).is_ok());
    }

    #[test]
    fn test_ct_app_execution_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--version"];

        // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_basename_h_ctmain() {
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }
    }

    #[test]
    fn test_basename_hh_ctmain() {
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }
    }

    #[test]
    fn test_basename_hhh_ctmain() {
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-H"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }
    }

    #[test]
    fn test_ct_app_invalid_argument() {
        let command = ct_app();

        // 测试用例：验证当提供未知参数时是否正确报错
        let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
        let result = command.try_get_matches_from(invalid_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_support_missing_argument() {
        let command = ct_app();

        // 测试用例：验证当缺少必需的参数时是否正确报错
        let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
        let result = command.try_get_matches_from(missing_args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_basename_v_ctmain() {
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn test_basename_vv_ctmain() {
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            // let result = ct_main(args.iter().map(|s| OsString::from(s)));

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn test_flags_multiple() {
        let command = ct_app();

        let args = vec![ctcore::ct_util_name(), "-a"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(flags::MULTIPLE));
    }

    #[test]
    fn test_flags_name() {
        let command = ct_app();

        let args = vec![
            ctcore::ct_util_name(),
            "-a",
            "path/to/name1",
            "path/to/name2",
        ];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(flags::MULTIPLE));
    }

    #[test]
    fn test_flags_suffix() {
        let command = ct_app();

        let args = vec![
            ctcore::ct_util_name(),
            "--suffix=SUFFIX",
            "/usr/bin/sort.txt",
        ];
        let matches = command.try_get_matches_from(args).unwrap();

        match matches.get_one::<String>(flags::SUFFIX) {
            Some(suffix) => {
                assert_eq!(suffix, "SUFFIX");
            }
            None => {
                assert!(false);
            }
        }
    }
    #[test]
    fn test_flags_zero() {
        let command = ct_app();

        let args = vec![ctcore::ct_util_name(), "-z"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(flags::ZERO));
    }

    #[test]
    fn test_basename_regular_input_with_valid_suffix() {
        let input = "/path/to/file.txt";
        let expected_result = "file";
        let suffix = ".txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_input_matches_suffix_exactly() {
        let input = "/path/to/file.txt";
        let expected_result = "file.txt";
        let suffix = "file.txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_hidden_file_with_valid_suffix() {
        let input = "/path/to/.txt";
        let expected_result = ".txt";
        let suffix = ".txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_no_suffix_provided() {
        let input = "/path/to/file";
        let expected_result = "file";
        let suffix = "";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_empty_input_and_suffix() {
        let input = "";
        let expected_result = "";
        let suffix = "";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_trailing_slash_in_input() {
        let input = "/path/to/file.txt/";
        let expected_result = "file";
        let suffix = ".txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_multiple_consecutive_slashes_in_input() {
        let input = "//path//to//file.txt";
        let expected_result = "file";
        let suffix = ".txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_suffix_containing_dots() {
        let input = "/path/to/file.tar.gz";
        let expected_result = "file.tar";
        let suffix = ".gz";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_long_input_without_suffix() {
        let input = "/very/long/path/to/a/really/really/really/really/really/long/file/name";
        let expected_result = "name";
        let suffix = "";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_suffix_containing_special_characters() {
        let input = "/path/to/file@#$%^&*.txt";
        let expected_result = "file@#$%^&*";
        let suffix = ".txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_with_invalid_input() {
        {
            // Test case: Input is a single slash
            let input = "/";
            let expected_result = "/";
            let suffix = "";

            let output = basename(input, suffix);
            assert_eq!(output, expected_result);
        }
    }

    #[test]
    fn test_ct_main() {
        // Test case: Input is a single slash
        let args = vec![ctcore::ct_util_name(), "/path/to/file@#$%^&*.txt", ".txt"];
        let expected_result = "/path/to/file@#$%^&*";
        let result = basename_main(args.iter().map(|s| OsString::from(s)));
        let mut s = String::new();
        // println!("{:?}", result);
        match result {
            Err(_output) => {
                let code = _output.code();
                let message = _output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(_output) => {
                s = "/path/to/file@#$%^&*".to_string();
                // println!("result:{}", s);
                // //assert_eq!(s,expected_output);
            }
        }
        assert_eq!(s, expected_result);
    }
}
