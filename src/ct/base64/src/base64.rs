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
use ct_base32::base_common;
use std::io::Read;
use std::io::stdin;

use crate::base_common::opt_flags;
use clap::Arg;
use clap::ArgAction;
use clap::Command;
use clap::crate_version;

use ctcore::{
    ct_encoding::Format, ct_error::CTResult, ct_format_usage, ct_help_about, ct_help_usage,
};

const BASE64_ABOUT: &str = ct_help_about!("base64.md");
const BASE64_USAGE: &str = ct_help_usage!("base64.md");

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    base64_main(args).map(|_| ())
}

pub fn base64_main(args: impl ctcore::Args) -> CTResult<String> {
    let format_mod = Format::Base64;

    let config_mod: base_common::BaseConfig =
        base_common::base_parsing_command_args(args, BASE64_ABOUT, BASE64_USAGE)?;

    let stdin_info = stdin();
    let mut input_info: Box<dyn Read> = base_common::get_base_input(&config_mod, &stdin_info)?;

    base_common::handle_base_input(
        &mut input_info,
        format_mod,
        config_mod.base_wrap_cols,
        config_mod.base_ignore_garbage,
        config_mod.base_decode,
    )
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = BASE64_ABOUT;
    let usage_description = ct_format_usage(BASE64_USAGE);

    let args = base64_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn base64_args_init() -> Vec<Arg> {
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
    use std::ffi::OsString;
    use std::fs;
    use std::fs::File;
    use std::io::{self, Write};

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
    fn test_base64_valid_ctmain() {
        let filename = "test_base64_valid_ctmain.txt";
        let content = "Test encode base64";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];
        let result = base64_main(args.iter().map(|s| OsString::from(s)));
        let expected_output = "VGVzdCBlbmNvZGUgYmFzZTY0";
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
    fn test_base64_valid_wrap_ctmain() {
        let filename = "test_base64_valid_wrap_ctmain.txt";
        let content = "Test test_valid_wrap_ctmain";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--wrap=8", filename];
        let result = base64_main(args.iter().map(|s| OsString::from(s)));
        let expected_output = "VGVzdCB0ZXN0X3ZhbGlkX3dyYXBfY3RtYWlu";
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
    fn test_base64_invalid_ctmain() {
        let filename = "test_base64_invalid_ctmain.txt";
        let content = "";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];

        let result = base64_main(args.iter().map(|s| OsString::from(s)));
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
    fn test_base64_wrap_invalid_ctmain() {
        let filename = "test_base64_wrap_invalid_ctmain.txt";
        let content = "";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--wrap=8", filename];

        let result = base64_main(args.iter().map(|s| OsString::from(s)));
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
    fn test_base64_invalid_input_ctmain() {
        let filename = "test_base64_invalid_input_ctmain.txt";
        let content = "KRSXG5BAMRSWG33EMUQGEYLTMUZTE===";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];
        let result = base64_main(args.iter().map(|s| OsString::from(s)));
        let expected_output = "S1JTWEc1QkFNUlNXRzMzRU1VUUdFWUxUTVVaVEU9PT0=";
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
    fn test_base64_decoding_d_ctmain() {
        let filename = "test_base64_decoding_d_ctmain.txt";
        let content = "VGVzdCBlbmNvZGUgYmFzZTY0"; /* Test decode base32 */
        let expected_output = "Test encode base64";
        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "-d", filename];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base64_main(args.iter().map(|s| OsString::from(s)));
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
    fn test_base64_decoding_ctmain() {
        let filename = "test_base64_decoding_ctmain.txt";
        let content = "VGVzdCBlbmNvZGUgYmFzZTY0";
        let expected_output = "Test encode base64";
        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--decode", filename];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base64_main(args.iter().map(|s| OsString::from(s)));
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
    fn test_base64_decoding_wrap_ctmain() {
        let filename = "test_base64_decoding_wrap_ctmain.txt";
        let content = "VGVzdCBlbmNvZGUgYmFzZTY0";
        let expected_output = "Test encode base64";
        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--wrap=64", "-d", filename];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base64_main(args.iter().map(|s| OsString::from(s)));
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
    fn test_base64_decoding_ignore_ctmain() {
        let filename = "test_base64_decoding_ignore_ctmain.txt";
        let expected_output = "";
        let content = "VGVzdCBlbmNvZGUgYmFzZTY0";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), " --ignore-garbage", "-d", filename];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base64_main(args.iter().map(|s| OsString::from(s)));
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
    fn test_base64_decoding_i_ctmain() {
        let filename = "test_base64_decoding_i_ctmain.txt";
        let content = "VGVzdCBlbmNvZGUgYmFzZTY0";

        // 创建文件并写入内容
        match create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), " -i", "-d", filename];
        //let args = ["--wrap", ""];
        let result: CTResult<String> = base64_main(args.iter().map(|s| OsString::from(s)));
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
    fn test_base64_decoding_wrap_ignore_ctmain() {
        let filename = "test_base64_decoding_wrap_ignore_ctmain.txt";
        let content = "VGVzdCBlbmNvZGUgYmFzZTY0";
        let expected_output = "";

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
        let result: CTResult<String> = base64_main(args.iter().map(|s| OsString::from(s)));
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
    fn test_base64_w_ctmain() {
        let filename = "test_base64_w_ctmain.txt";
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
    fn test_base64_i_ctmain() {
        // 测试用例1：
        let args = ["--ignore-garbage", ""];
        let result = ctmain(args.iter().map(|s| OsString::from(s)));
        println!("{}", result);
        assert_eq!(result, 1);

        let filename = "test_base64_i_ctmain.txt";
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
    fn test_base64_h_ctmain() {
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
    fn test_base64_hh_ctmain() {
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
    fn test_base64_v_ctmain() {
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
    fn test_base64_vv_ctmain() {
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
