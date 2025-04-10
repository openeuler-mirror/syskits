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
use rust_i18n::t;
use std::ffi::OsString;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use std::io::Read;
use std::io::stdin;

use crate::base_common::opt_flags;

use clap::Arg;
use clap::ArgAction;
use clap::Command;
use clap::crate_version;
use ctcore::{Tool, ct_encoding::Format, ct_error::CTResult};
use sys_locale::get_locale;

pub mod base_common;

#[derive(Default)]
pub struct Base32;
impl Tool for Base32 {
    fn name(&self) -> &'static str {
        "base32"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        base32_main(args.iter().cloned()).map(|_| ())
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    base32_main(args).map(|_| ())
}

pub fn base32_main(args: impl ctcore::Args) -> CTResult<String> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let format_mod = Format::Base32;
    let base32_about = t!("base32.about");
    let base32_usage = t!("base32.usage");

    let config_info: base_common::BaseConfig =
        base_common::base_parsing_command_args(args, base32_about, base32_usage)?;

    let stdin_info = stdin();
    let mut input_info: Box<dyn Read> = base_common::get_base_input(&config_info, &stdin_info)?;

    base_common::handle_base_input(
        &mut input_info,
        format_mod,
        config_info.base_wrap_cols,
        config_info.base_ignore_garbage,
        config_info.base_decode,
    )
}

pub fn ct_app() -> Command {
    let util_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("base32.about");
    let usage_description = t!("base32.usage");

    let args = base32_args_init();

    Command::new(util_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn base32_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(opt_flags::BASE_DECODE)
            .short('d')
            .long(opt_flags::BASE_DECODE)
            .help("decode data")
            .action(ArgAction::SetTrue)
            .overrides_with(opt_flags::BASE_DECODE),
        Arg::new(opt_flags::BASE_IGNORE_GARBAGE)
            .short('i')
            .long(opt_flags::BASE_IGNORE_GARBAGE)
            .help("when decoding, ignore non-alphabetic characters")
            .action(ArgAction::SetTrue)
            .overrides_with(opt_flags::BASE_IGNORE_GARBAGE),
        Arg::new(opt_flags::BASE_WRAP)
            .short('w')
            .long(opt_flags::BASE_WRAP)
            .value_name("COLS")
            .help("wrap encoded lines after COLS character (default 76, 0 to disable wrapping)")
            .overrides_with(opt_flags::BASE_WRAP),
        Arg::new(opt_flags::BASE_FILE)
            .index(1)
            .action(clap::ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];
    args
}

#[cfg(test)]
mod test {
    use super::*;
    use clap::error::ErrorKind;
    use std::fs;
    use std::fs::File;
    use std::io::{self, Write};

    #[test]
    fn test_tool_implementation() {
        let tool = Base32::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "base32");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("base32"));

        // 测试 execute 方法
        let args = vec![OsString::from("base32"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err()); // base32 命令可以接受标准输入，所以不带参数也可以执行
    }

    // 创建文件并写入内容
    fn create_file_with_content(filename: &str, content: &str) -> io::Result<()> {
        let mut file = File::create(filename)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    // 删除指定文件
    fn delete_file(filename: &str) -> io::Result<()> {
        fs::remove_file(filename)?;
        Ok(())
    }

    #[test]
    fn test_valid_ctmain() {
        let filename = "test_valid_ctmain.txt";
        let content = "Test decode base32";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];
        let result = base32_main(args.iter().map(|s| OsString::from(s)));
        let expected_output = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE===";
        let mut s = String::new();
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
                println!("result:{}", s);
                println!("{}", expected_output);
                //assert_eq!(s,expected_output);
            }
        }
        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }
    #[test]
    fn test_valid_wrap_ctmain() {
        let filename = "test_valid_wrap_ctmain.txt";
        let content = "Test decode base32 Test decode base32";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--wrap=8", filename];
        let result = base32_main(args.iter().map(|s| OsString::from(s)));
        let expected_output = "KRSXG5BAMRSWG33EMUQGEYLTMUZTEICUMVZXIIDEMVRW6ZDFEBRGC43FGMZA====";
        let mut s = String::new();
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
                println!("result:{}", s);
                println!("{}", expected_output);
                //assert_eq!(s,expected_output);
            }
        }
        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_invalid_ctmain() {
        let filename = "test_invalid_ctmain.txt";
        let content = "";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];

        let result = base32_main(args.iter().map(|s| OsString::from(s)));
        let expected_output = ""; // 预期输出结果为空
        let mut s = String::new();
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
                println!("result:{}", s);
                println!("{}", expected_output);
                //assert_eq!(s,expected_output);
            }
        }
        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_invalid_wrap_ctmain() {
        let filename = "test_invalid_ctmain.txt";
        let content = "";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--wrap=8", filename];

        let result = base32_main(args.iter().map(|s| OsString::from(s)));
        let expected_output = ""; // 预期输出结果为空
        let mut s = String::new();
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
                println!("result:{}", s);
                println!("{}", expected_output);
                //assert_eq!(s,expected_output);
            }
        }
        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_invalid_input_ctmain() {
        let filename = "test_invalid_input_ctmain.txt";
        let content = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE===";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];
        let result = base32_main(args.iter().map(|s| OsString::from(s)));
        let expected_output = "JNJFGWCHGVBECTKSKNLUOMZTIVGVKUKHIVMUYVCNKVNFIRJ5HU6Q====";
        let mut s = String::new();
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
                println!("result:{}", s);
                println!("{}", expected_output);
                //assert_eq!(s,expected_output);
            }
        }
        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_decoding_d_ctmain() {
        let filename = "test_decoding_d_ctmain.txt";
        let content = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE==="; /* Test decode base32 */

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "-d", filename];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base32_main(args.iter().map(|s| OsString::from(s)));
        let mut s = String::new();
        let expected_output = "Test decode base32";
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
            }
        }

        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }

        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_decoding_ctmain() {
        let filename = "test_decoding_ctmain.txt";
        let content = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE==="; /* Test decode base32 */

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--decode", filename];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base32_main(args.iter().map(|s| OsString::from(s)));
        let mut s = String::new();
        let expected_output = "Test decode base32";
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
            }
        }

        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }

        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_decoding_wrap_ctmain() {
        let filename = "test_decoding_wrap_ctmain.txt";
        let content = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE==="; /* Test decode base32 */

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--wrap=64", "-d", filename];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base32_main(args.iter().map(|s| OsString::from(s)));
        let mut s = String::new();
        let expected_output = "Test decode base32";
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
            }
        }

        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }

        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_decoding_ignore_ctmain() {
        let filename = "test_decoding_ignore_ctmain.txt";
        let content = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE==="; /* Test decode base32 */

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), " --ignore-garbage", "-d", filename];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base32_main(args.iter().map(|s| OsString::from(s)));
        let mut s = String::new();
        let expected_output = "";
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
            }
        }

        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }

        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_decoding_i_ctmain() {
        let filename = "test_decoding_i_ctmain.txt";
        let content = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE==="; /* Test decode base32 */

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), " -i", "-d", filename];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base32_main(args.iter().map(|s| OsString::from(s)));
        let mut s = String::new();
        let expected_output = "";
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
            }
        }

        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }

        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_decoding_wrap_ignore_ctmain() {
        let filename = "test_decoding_wrap_ignore_ctmain.txt";
        let content = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE==="; /* Test decode base32 */

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--wrap",
            "--ignore-garbage",
            "-d",
            filename,
        ];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base32_main(args.iter().map(|s| OsString::from(s)));
        let mut s = String::new();
        let expected_output = "";
        // 使用模式匹配提取字段值
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
            }
        }

        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }

        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_w_ctmain() {
        let filename = "test_w_ctmain.txt";
        let content = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE==="; /* Test decode base32 */

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--wrap=64", filename];
        //let args = ["--wrap", ""];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        println!("{}", result);
        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }

        assert_eq!(result, 0);
    }
    #[test]
    fn test_i_ctmain() {
        // 测试用例1：
        let args = ["--ignore-garbage", ""];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        println!("{}", result);
        assert_eq!(result, 1);

        let filename = "test_i_ctmain.txt";
        let content = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE==="; /* Test decode base32 */

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--wrap=64",
            "--ignore-garbage",
            filename,
        ];
        //let args = ["--wrap", ""];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        println!("{}", result);

        // 删除文件
        match delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }

        assert_eq!(result, 0);
    }

    #[test]
    fn test_h_ctmain() {
        {
            let args = ["--help", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            println!("{}", result);
            assert_eq!(result, 1);
        }
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            // let result = ct_main(args.iter().map(|s| OsString::from(s)));

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }
    }
    #[test]
    fn test_hh_ctmain() {
        {
            let args = ["-h", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            println!("{}", result);
            assert_eq!(result, 1);
        }

        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            // let result = ct_main(args.iter().map(|s| OsString::from(s)));

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }
    }

    #[test]
    fn test_v_ctmain() {
        {
            let args = ["--version", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            println!("{}", result);
            assert_eq!(result, 1);
        }
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            // let result = ct_main(args.iter().map(|s| OsString::from(s)));

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn test_vv_ctmain() {
        {
            let args = ["-V", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            println!("{}", result);
            assert_eq!(result, 1);
        }
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            // let result = ct_main(args.iter().map(|s| OsString::from(s)));

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
    }
}
