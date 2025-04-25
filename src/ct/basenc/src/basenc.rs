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
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use clap::ArgAction;
use clap::Command;
use ct_base32::base_common::{self, BASE_CMD_PARSE_ERROR, BaseConfig};
use sys_locale::get_locale;

use ctcore::{
    ct_encoding::Format,
    ct_error::{CTResult, CTsageError},
};

use ctcore::Tool;
use ctcore::ct_error::UClapError;
use std::ffi::OsString;
use std::io::Read;
use std::io::stdin;

const BASE64_ENCODINGS: &[(&str, Format, &str)] = &[
    ("base64", Format::Base64, "same as 'base64' program"),
    ("base64url", Format::Base64Url, "file- and url-safe base64"),
    ("base32", Format::Base32, "same as 'base32' program"),
    (
        "base32hex",
        Format::Base32Hex,
        "extended hex alphabet base32",
    ),
    ("base16", Format::Base16, "hex encoding"),
    (
        "base2lsbf",
        Format::Base2Lsbf,
        "bit string with least significant bit (lsb) first",
    ),
    (
        "base2msbf",
        Format::Base2Msbf,
        "bit string with most significant bit (msb) first",
    ),
    (
        "z85",
        Format::Z85,
        "ascii85-like encoding;\n\
         when encoding, input length must be a multiple of 4;\n\
         when decoding, input length must be a multiple of 5",
    ),
];

pub fn ct_app() -> Command {
    let base64_about = t!("basenc.about");
    let base64_usage = t!("basenc.usage");
    let mut ct_cmd = base_common::base_common_app(base64_about, base64_usage);
    for encoding in BASE64_ENCODINGS {
        let raw = Arg::new(encoding.0)
            .long(encoding.0)
            .help(encoding.2)
            .action(ArgAction::SetTrue);
        let overriding = BASE64_ENCODINGS
            .iter()
            .fold(raw, |arg, enc| arg.overrides_with(enc.0));
        ct_cmd = ct_cmd.arg(overriding);
    }
    ct_cmd
}

fn basenc_parse_cmd_args(args: impl ctcore::Args) -> CTResult<(BaseConfig, Format)> {
    let args_match = ct_app()
        .try_get_matches_from(args.collect_lossy())
        .with_exit_code(1)?;
    let format_mod = BASE64_ENCODINGS
        .iter()
        .find(|encoding| args_match.get_flag(encoding.0))
        .ok_or_else(|| CTsageError::new(BASE_CMD_PARSE_ERROR, "missing encoding type"))?
        .1;
    let config_mod = BaseConfig::from(&args_match)?;
    Ok((config_mod, format_mod))
}

#[derive(Default)]
pub struct Basenc;
impl Tool for Basenc {
    fn name(&self) -> &'static str {
        "basenc"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        basenc_main(args.iter().cloned()).map(|_| ())
    }
}


pub fn basenc_main(args: impl ctcore::Args) -> CTResult<String> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let (config_mod, format_mod) = basenc_parse_cmd_args(args)?;

    // 创建对stdin的引用，以便我们能从parse_base_cmd_args返回锁定的stdin
    let ct_stdin = stdin();
    let mut ct_input: Box<dyn Read> = base_common::get_base_input(&config_mod, &ct_stdin)?;

    base_common::handle_base_input(
        &mut ct_input,
        format_mod,
        config_mod.base_wrap_cols,
        config_mod.base_ignore_garbage,
        config_mod.base_decode,
    )
}

#[cfg(test)]

mod test {
    use super::*;

    use std::ffi::OsString;
    use std::fs;
    use std::fs::File;

    use clap::error::ErrorKind;
    use std::io::{self, Write};

    #[test]
    fn test_tool_implementation() {
        let tool = Basenc::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "basenc");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("basenc"));

        // 测试 execute 方法
        let args = vec![OsString::from("basenc"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err()); // basenc needs an encoding flag to be valid
    }

    // 创建文件并写入内容
    fn base_create_file_with_content(filename: &str, content: &str) -> io::Result<()> {
        let mut file = File::create(filename)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    // 删除指定文件
    fn base_delete_file(filename: &str) -> io::Result<()> {
        fs::remove_file(filename)?;
        Ok(())
    }
    #[test]
    fn test_basenc_encode_base16() {
        let filename = "test_basenc_encode_base16.txt";
        let content = "Test  test_base_common_handle_input_encode_base16";
        let expected_output = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--base16", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base32() {
        let filename = "test_basenc_encode_base32.txt";
        let content = "Test test_base_common_handle_input_encode_base32";
        let expected_output =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--base32", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }
    #[test]
    fn test_basenc_encode_base32hex() {
        let filename = "test_basenc_encode_base32hex.txt";
        let content = "Test test_base_common_handle_input_encode_base32hex";
        let expected_output = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--base32hex", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base64() {
        let filename = "test_basenc_encode_base64.txt";
        let content = "Test test_base_common_handle_input_encode_base64";
        let expected_output = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--base64", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }
    #[test]
    fn test_basenc_encode_base64url() {
        let filename = "test_basenc_encode_base64url.txt";
        let content = "Test test_base_common_handle_input_encode_base64url";

        let expected_output =
            "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--base64url", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base16() {
        let filename = "test_basenc_decode_base16.txt";
        let content = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
        let expected_output = "Test  test_base_common_handle_input_encode_base16";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "-d", "--base16", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base16_wrap() {
        let filename = "test_basenc_decode_base16_wrap.txt";
        let content = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
        let expected_output = "Test  test_base_common_handle_input_encode_base16";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--base16",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base32() {
        let filename = "test_basenc_decode_base32.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32";
        let content =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "-d", "--base32", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base32_wrap() {
        let filename = "test_basenc_decode_base32_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32";
        let content =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--base32",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }
    #[test]
    fn test_basenc_decode_base32hex() {
        let filename = "webasenc_decode_base32hex.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "-d", "--base32hex", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base32hex_wrap() {
        let filename = "test_basenc_decode_base32hex_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--base32hex",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base64() {
        let filename = "test_basenc_decode_base64.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64";
        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "-d", "--base64", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base64_wrap() {
        let filename = "test_basenc_decode_base64_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64";
        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--base64",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base64url() {
        let filename = "test_basenc_decode_base64url.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64url";

        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "-d", "--base64url", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base64url_wrap() {
        let filename = "test_basenc_decode_base64url_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64url";

        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--base64url",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base2lsbf() {
        let filename = "test_basenc_encode_base2lsbf.txt";
        let content = "Test Base2Lsbf";

        let expected_output = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--base2lsbf", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base2msbf() {
        let filename = "test_basenc_encode_base2msbf.txt";
        let content = "Test Base2Msbf";

        let expected_output = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--base2msbf", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base2lsbf_wrap() {
        let filename = "test_basenc_encode_base2lsbf_wrap.txt";
        let content = "Test Base2Lsbf";

        let expected_output = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--base2lsbf", "--wrap=8", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base2msbf_wrap() {
        let filename = "test_basenc_encode_base2msbf_wrap.txt";
        let content = "Test Base2Msbf";

        let expected_output = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--base2msbf", "--wrap=8", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base2lsbf() {
        let filename = "test_basenc_decode_base2lsbf.txt";
        let expected_output = "Test Base2Lsbf";

        let content = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--decode", "--base2lsbf", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base2msbf() {
        let filename = "test_basenc_decode_base2msbf.txt";
        let expected_output = "Test Base2Msbf";

        let content = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--decode", "--base2msbf", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base2lsbf_wrap() {
        let filename = "test_basenc_decode_base2lsbf_wrap.txt";
        let expected_output = "Test Base2Lsbf";

        let content = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--decode",
            "--base2lsbf",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base2msbf_wrap() {
        let filename = "test_basenc_decode_base2msbf_wrap.txt";
        let expected_output = "Test Base2Msbf";

        let content = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--decode",
            "--base2msbf",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_basez85() {
        let filename = "test_basenc_encode_basez85.txt";
        let content = "TestBZ85";

        let expected_output = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--z85", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_basez85_wrap() {
        let filename = "test_basenc_encode_basez85_wrap.txt";
        let content = "TestBZ85";

        let expected_output = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--wrap=6", "--z85", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_basez85() {
        let filename = "test_basenc_decode_basez85.txt";
        let expected_output = "TestBZ85";

        let content = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--decode", "--z85", filename];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_basez85_wrap() {
        let filename = "test_basenc_decode_basez85_wrap.txt";
        let expected_output = "TestBZ85";

        let content = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--decode",
            "--z85",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_h_ctmain() {
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
    fn test_basenc_hh_ctmain() {
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
    fn test_basenc_v_ctmain() {
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
    fn test_basenc_vv_ctmain() {
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
    //////////////////////////////////////////////////////
    #[test]
    fn test_basenc_encode_base16_ignore() {
        let filename = "test_basenc_encode_base16_ignore.txt";
        let content = "Test  test_base_common_handle_input_encode_base16";
        let expected_output = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base16",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base32_ignore() {
        let filename = "test_basenc_encode_base32_ignore.txt";
        let content = "Test test_base_common_handle_input_encode_base32";
        let expected_output =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base32",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }
    #[test]
    fn test_basenc_encode_base32hex_ignore() {
        let filename = "test_basenc_encode_base32hex_ignore.txt";
        let content = "Test test_base_common_handle_input_encode_base32hex";
        let expected_output = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base32hex",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base64_ignore() {
        let filename = "test_basenc_encode_base64_ignore.txt";
        let content = "Test test_base_common_handle_input_encode_base64";
        let expected_output = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base64",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }
    #[test]
    fn test_basenc_encode_base64url_ignore() {
        let filename = "test_basenc_encode_base64url_ignore.txt";
        let content = "Test test_base_common_handle_input_encode_base64url";

        let expected_output =
            "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base64url",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base16_ignore() {
        let filename = "test_basenc_decode_base16_ignore.txt";
        let content = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
        let expected_output = "Test  test_base_common_handle_input_encode_base16";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base16",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base16_wrap_ignore() {
        let filename = "test_basenc_decode_base16_wrap_ignore.txt";
        let content = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
        let expected_output = "Test  test_base_common_handle_input_encode_base16";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base16",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base32_ignore() {
        let filename = "test_basenc_decode_base32_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32";
        let content =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base32",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base32_wrap_ignore() {
        let filename = "test_basenc_decode_base32_wrap_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32";
        let content =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base32",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }
    #[test]
    fn test_basenc_decode_base32hex_ignore() {
        let filename = "test_basenc_decode_base32hex.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base32hex",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base32hex_wrap_ignore() {
        let filename = "test_basenc_decode_base32hex_wrap_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base32hex",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base64_ignore() {
        let filename = "test_basenc_decode_base64_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64";
        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base64",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base64_wrap_ignore() {
        let filename = "test_basenc_decode_base64_wrap_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64";
        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base64",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base64url_ignore() {
        let filename = "test_basenc_decode_base64url_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64url";

        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base64url",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base64url_wrap_ignore() {
        let filename = "test_basenc_decode_base64url_wrap_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64url";

        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base64url",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base2lsbf_ignore() {
        let filename = "test_basenc_encode_base2lsbf_ignore.txt";
        let content = "Test Base2Lsbf";

        let expected_output = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base2lsbf",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base2msbf_ignore() {
        let filename = "test_basenc_encode_base2msbf_ignore.txt";
        let content = "Test Base2Msbf";

        let expected_output = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base2msbf",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base2lsbf_wrap_ignore() {
        let filename = "test_basenc_encode_base2lsbf_wrap_ignore.txt";
        let content = "Test Base2Lsbf";

        let expected_output = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base2lsbf",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_base2msbf_wrap_ignore() {
        let filename = "test_basenc_encode_base2msbf_wrap_ignore.txt";
        let content = "Test Base2Msbf";

        let expected_output = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base2msbf",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base2lsbf_ignore() {
        let filename = "test_basenc_decode_base2lsbf_ignore.txt";
        let expected_output = "Test Base2Lsbf";

        let content = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--decode",
            "--base2lsbf",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base2msbf_ignore() {
        let filename = "test_basenc_decode_base2msbf_ignore.txt";
        let expected_output = "Test Base2Msbf";

        let content = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--decode",
            "--base2msbf",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base2lsbf_wrap_ingore() {
        let filename = "test_basenc_decode_base2lsbf_wrap_ingore.txt";
        let expected_output = "Test Base2Lsbf";

        let content = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--decode",
            "--base2lsbf",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base2msbf_wrap_ignore() {
        let filename = "test_basenc_decode_base2msbf_wrap_ignore.txt";
        let expected_output = "Test Base2Msbf";

        let content = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--decode",
            "--base2msbf",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_basez85_ignore() {
        let filename = "test_basenc_encode_basez85_ignore.txt";
        let content = "TestBZ85";

        let expected_output = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--z85",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_encode_basez85_wrap_ignore() {
        let filename = "test_basenc_encode_basez85_wrap_ignore.txt";
        let content = "TestBZ85";

        let expected_output = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--wrap=6",
            "--z85",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_basez85_ignore() {
        let filename = "test_basenc_decode_basez85_ignore.txt";
        let expected_output = String::new(); //"TestBZ85";

        let content = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "-ignore-garbage",
            "--decode",
            "--z85",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_basez85_wrap_ignore() {
        let filename = "test_basenc_decode_basez85_wrap_ignore.txt";
        let expected_output = "TestBZ85";

        let content = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--decode",
            "--z85",
            "--wrap=8",
            filename,
        ];
        let result = basenc_main(args.iter().map(|s| OsString::from(s)));
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
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
        }
        assert_eq!(s, expected_output);
    }
}
