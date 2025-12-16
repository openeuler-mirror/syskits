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

use std::io::stdout;
use std::io::Read;
use std::io::Write;

use ctcore::ct_display::Quotable;
use ctcore::ct_encoding::wrap_print;
use ctcore::ct_encoding::CtEncodeError;
use ctcore::ct_encoding::Data;
use ctcore::ct_encoding::Format;
use ctcore::ct_error::CTResult;
use ctcore::ct_error::CTsageError;
use ctcore::ct_error::CtSimpleError;
use ctcore::ct_error::FromIo;
use ctcore::ct_format_usage;

use std::fs::File;
use std::io::BufReader;
use std::io::Stdin;
use std::path::Path;

use clap::crate_version;
use clap::Arg;
use clap::ArgAction;
use clap::Command;

pub static BASE_CMD_PARSE_ERROR: i32 = 1;

// Config.
pub struct BaseConfig {
    pub base_decode: bool,
    pub base_ignore_garbage: bool,
    pub base_wrap_cols: Option<usize>,
    pub base_to_read: Option<String>,
}

pub mod opt_flags {
    pub static BASE_DECODE: &str = "decode";
    pub static BASE_WRAP: &str = "wrap";
    pub static BASE_IGNORE_GARBAGE: &str = "ignore-garbage";
    pub static BASE_FILE: &str = "file";
}

impl BaseConfig {
    pub fn from(options: &clap::ArgMatches) -> CTResult<Self> {
        let f: Option<String> = match options.get_many::<String>(opt_flags::BASE_FILE) {
            Some(mut var) => {
                let path_name = var.next().unwrap();
                if let Some(extra_operand) = var.next() {
                    return Err(CTsageError::new(
                        BASE_CMD_PARSE_ERROR,
                        format!("extra operand {}", extra_operand.quote(),),
                    ));
                }

                match path_name.as_ref() {
                    "-" => None,
                    _ => {
                        if !Path::exists(Path::new(path_name)) {
                            return Err(CtSimpleError::new(
                                BASE_CMD_PARSE_ERROR,
                                format!("{}: No such file or directory", path_name.maybe_quote()),
                            ));
                        }
                        Some(path_name.clone())
                    }
                }
            }
            None => None,
        };

        let cols = options
            .get_one::<String>(opt_flags::BASE_WRAP)
            .map(|num| {
                num.parse::<usize>().map_err(|_| {
                    CtSimpleError::new(
                        BASE_CMD_PARSE_ERROR,
                        format!("invalid wrap size: {}", num.quote()),
                    )
                })
            })
            .transpose()?;

        Ok(Self {
            base_decode: options.get_flag(opt_flags::BASE_DECODE),
            base_ignore_garbage: options.get_flag(opt_flags::BASE_IGNORE_GARBAGE),
            base_wrap_cols: cols,
            base_to_read: f,
        })
    }
}

pub fn base_parsing_command_args(
    base_args: impl ctcore::Args,
    base_about: &'static str,
    base_usage: &str,
) -> CTResult<BaseConfig> {
    let command = base_common_app(base_about, base_usage);
    BaseConfig::from(&command.try_get_matches_from(base_args)?)
}

pub fn base_common_app(about: &'static str, usage: &str) -> Command {
    let util_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = about;
    let usage_description = ct_format_usage(usage);

    let args = base_args_init();

    Command::new(util_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn base_args_init() -> Vec<Arg> {
    let base_args = vec![
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
    base_args
}

pub fn get_base_input<'a>(
    ct_config: &BaseConfig,
    ct_stdin_ref: &'a Stdin,
) -> CTResult<Box<dyn Read + 'a>> {
    match &ct_config.base_to_read {
        Some(base_name) => {
            let file_buf = File::open(Path::new(base_name))
                .map_err_context(|| base_name.maybe_quote().to_string())?;
            Ok(Box::new(BufReader::new(file_buf))) //作为 Box<dyn Read> 类型转换
        }
        None => {
            Ok(Box::new(ct_stdin_ref.lock())) //作为 Box<dyn Read> 类型转换
        }
    }
}

pub fn handle_base_input<R: Read>(
    ct_input: &mut R,
    ct_format: Format,
    ct_line_wrap: Option<usize>,
    ct_ignore_garbage: bool,
    ct_decode: bool,
) -> CTResult<String> {
    let mut input_data = Data::new(ct_input, ct_format).ignore_garbage(ct_ignore_garbage);
    if let Some(wrap) = ct_line_wrap {
        input_data = input_data.line_wrap(wrap);
    }

    if ct_decode {
        // println!("--------------- decode ----------------");
        match input_data.decode() {
            Ok(s) => {
                // 抑制此警告，因为我们希望显示错误消息
                #[allow(clippy::question_mark)]
                if stdout().write_all(&s).is_err() {
                    // 在Windows控制台中，尝试写出无效UTF-8编码会引发错误
                    return Err(CtSimpleError::new(1, "error: cannot write non-utf8 data"));
                }

                fn convert_vec_to_string_lossy(vec: Vec<u8>) -> String {
                    String::from_utf8_lossy(&vec).into_owned()
                }

                // // 示例
                // let bytes: Vec<u8> = vec![72, 101, 108, 108, 111, 239]; // 含有无效字节
                let string = convert_vec_to_string_lossy(s);
                // // println!("Converted string (lossy): {}", string);
                // let ss = String::new();
                // let ss = "test";

                Ok(string)
            }
            Err(_) => Err(CtSimpleError::new(1, "error: invalid input")),
        }
    } else {
        // println!("--------------- encode ----------------");
        match input_data.encode() {
            Ok(s) => {
                wrap_print(&input_data, &s);
                Ok(s)
            }
            Err(CtEncodeError::InvalidInput) => Err(CtSimpleError::new(1, "error: invalid input")),
            Err(_) => Err(CtSimpleError::new(
                1,
                "error: invalid input (length must be multiple of 4 characters)",
            )),
        }
    }
}

#[cfg(test)]

mod test {
    use super::*;

    use crate::{base_common, BASE32_ABOUT, BASE32_USAGE};
    use ctcore::ct_encoding::Format;
    use std::ffi::OsString;
    use std::fs;
    use std::fs::File;
    use std::io::stdin;
    use std::io::{self, Write};

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
    fn test_base_common_handle_input_encode_base16() {
        let filename = "base_common_Base16.txt";
        let content = "Test  test_base_common_handle_input_encode_base16";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];
        let format = Format::Base16;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );
        let expected_output = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
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
    fn test_base_common_handle_input_encode_base32() {
        let filename = "base_common_Base32.txt";
        let content = "Test test_base_common_handle_input_encode_base32";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];
        let format = Format::Base32;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );
        let expected_output =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
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
    fn test_base_common_handle_input_encode_base32hex() {
        let filename = "base_common_Base32Hex.txt";
        let content = "Test test_base_common_handle_input_encode_base32hex";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];
        let format = Format::Base32Hex;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );
        let expected_output = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";
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
    fn test_base_common_handle_input_encode_base64() {
        let filename = "base_common_Base64.txt";
        let content = "Test test_base_common_handle_input_encode_base64";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];
        let format = Format::Base64;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );
        let expected_output = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
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
    fn test_base_common_handle_input_encode_base64url() {
        let filename = "base_common_Base64Url.txt";
        let content = "Test test_base_common_handle_input_encode_base64url";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), filename];
        let format = Format::Base64Url;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );
        let expected_output =
            "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
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
    fn test_base_common_handle_input_decode_base16() {
        let filename = "base_common_decode_Base16.txt";
        let content =
        "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "-d", filename];
        let format = Format::Base16;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );
        let expected_output = "Test  test_base_common_handle_input_encode_base16";
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
    fn test_base_common_handle_input_decode_base16_wrap() {
        let filename = "base_common_decode_Base16_wrap.txt";
        let content =
            "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let args = vec![ctcore::ct_util_name(), "--decode", "--wrap=8", filename];
        let format = Format::Base16;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );
        let expected_output = "Test  test_base_common_handle_input_encode_base16";
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
    fn test_base_common_handle_input_decode_base32() {
        let filename = "base_common_decode_Base32.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32";
        let content =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "-d", filename];
        let format = Format::Base32;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );

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
    fn test_base_common_handle_input_decode_base32_wrap() {
        let filename = "base_common_decode_Base32_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32";
        let content =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--decode", "--wrap=8", filename];
        let format = Format::Base32;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );

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
    fn test_base_common_handle_input_decode_base32hex() {
        let filename = "base_common_decode_Base32hex.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content =
            "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "-d", filename];
        let format = Format::Base32Hex;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );

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
    fn test_base_common_handle_input_decode_base32hex_wrap() {
        let filename = "base_common_decode_Base32hex_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content =
            "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--decode", "--wrap=8", filename];
        let format = Format::Base32Hex;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(|s| OsString::from(s)),
            BASE32_ABOUT,
            BASE32_USAGE,
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let result = base_common::handle_base_input(
            &mut input,
            format,
            config.base_wrap_cols,
            config.base_ignore_garbage,
            config.base_decode,
        );

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

 
}