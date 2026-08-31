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

use std::io::Read;
use std::io::Write;

use ctcore::ct_display::Quotable;
use ctcore::ct_encoding::CtEncodeError;
use ctcore::ct_encoding::Data;
use ctcore::ct_encoding::Format;
use ctcore::ct_encoding::wrap_write;
use ctcore::ct_error::CTResult;
use ctcore::ct_error::CTsageError;
use ctcore::ct_error::CtSimpleError;
use ctcore::ct_error::FromIo;

use std::fs::File;
use std::io::BufReader;
use std::io::Stdin;
use std::path::Path;

use clap::Arg;
use clap::ArgAction;
use clap::Command;
use clap::crate_version;

pub static BASE_CMD_PARSE_ERROR: i32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaseCoreOutput {
    pub bytes: Vec<u8>,
    pub text: String,
}

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
    base_about: String,
    base_usage: String,
) -> CTResult<BaseConfig> {
    let command = base_common_app(base_about, base_usage);
    BaseConfig::from(&command.try_get_matches_from(base_args)?)
}

pub fn base_common_app(about: String, usage: String) -> Command {
    let util_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = about;
    let usage_description = usage;

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
    _ct_stdin_ref: &'a Stdin,
) -> CTResult<Box<dyn Read + 'a>> {
    match &ct_config.base_to_read {
        Some(base_name) => {
            let file_buf = File::open(Path::new(base_name))
                .map_err_context(|| base_name.maybe_quote().to_string())?;
            Ok(Box::new(BufReader::new(file_buf))) //作为 Box<dyn Read> 类型转换
        }
        None => Ok(ctcore::ct_io::stdin_reader_box()),
    }
}

pub fn handle_base_input<R: Read, W: Write>(
    ct_input: &mut R,
    mut writer: W,
    ct_format: Format,
    ct_line_wrap: Option<usize>,
    ct_ignore_garbage: bool,
    ct_decode: bool,
) -> CTResult<()> {
    let mut input_data = Data::new(ct_input, ct_format).ignore_garbage(ct_ignore_garbage);
    if let Some(wrap) = ct_line_wrap {
        input_data = input_data.line_wrap(wrap);
    }

    if ct_decode {
        match input_data.decode(&mut writer) {
            Ok(_) => Ok(()),
            Err(_) => Err(CtSimpleError::new(1, "invalid input")),
        }
    } else {
        match input_data.encode(&mut writer) {
            Ok(_) => Ok(()),
            Err(CtEncodeError::InvalidInput) => Err(CtSimpleError::new(1, "invalid input")),
            Err(_) => Err(CtSimpleError::new(
                1,
                "invalid input (length must be multiple of 4 characters)",
            )),
        }
    }
}

pub fn handle_base_input_to_writer<R: Read, W: Write>(
    ct_input: &mut R,
    ct_format: Format,
    ct_line_wrap: Option<usize>,
    ct_ignore_garbage: bool,
    ct_decode: bool,
    writer: &mut W,
) -> CTResult<()> {
    handle_base_input(
        ct_input,
        writer,
        ct_format,
        ct_line_wrap,
        ct_ignore_garbage,
        ct_decode,
    )
}

pub fn handle_base_input_core<R: Read>(
    ct_input: &mut R,
    ct_format: Format,
    ct_line_wrap: Option<usize>,
    ct_ignore_garbage: bool,
    ct_decode: bool,
) -> CTResult<BaseCoreOutput> {
    if ct_decode {
        let mut input_data = Data::new(ct_input, ct_format).ignore_garbage(ct_ignore_garbage);
        let mut bytes = Vec::new();
        match input_data.decode(&mut bytes) {
            Ok(_) => Ok(BaseCoreOutput {
                text: String::from_utf8_lossy(&bytes).into_owned(),
                bytes,
            }),
            Err(_) => Err(CtSimpleError::new(1, "error: invalid input")),
        }
    } else {
        let mut raw = Vec::new();
        ct_input
            .read_to_end(&mut raw)
            .map_err(|_| CtSimpleError::new(1, "error: invalid input"))?;

        match ctcore::ct_encoding::encode(ct_format, &raw) {
            Ok(text) => {
                let mut bytes = Vec::new();
                wrap_write(&mut bytes, ct_line_wrap.unwrap_or(76), &text)
                    .map_err_context(|| "error: invalid input".to_string())?;
                Ok(BaseCoreOutput { text, bytes })
            }
            Err(CtEncodeError::InvalidInput) => Err(CtSimpleError::new(1, "error: invalid input")),
            Err(_) => Err(CtSimpleError::new(
                1,
                "invalid input (length must be multiple of 4 characters)",
            )),
        }
    }
}

const STREAM_READ_BUF_SIZE: usize = 64 * 1024;

pub fn handle_base_input_streaming_to_writer<R: Read, W: Write>(
    ct_input: &mut R,
    ct_format: Format,
    ct_line_wrap: Option<usize>,
    ct_ignore_garbage: bool,
    ct_decode: bool,
    writer: &mut W,
) -> CTResult<()> {
    if matches!(ct_format, Format::Base58) {
        return handle_base_input(
            ct_input,
            writer,
            ct_format,
            ct_line_wrap,
            ct_ignore_garbage,
            ct_decode,
        );
    }

    if ct_decode {
        stream_decode_to_writer(ct_input, ct_format, ct_ignore_garbage, writer)
    } else {
        stream_encode_to_writer(ct_input, ct_format, ct_line_wrap.unwrap_or(76), writer)
    }
}

fn stream_encode_to_writer<R: Read, W: Write>(
    ct_input: &mut R,
    ct_format: Format,
    ct_line_wrap: usize,
    writer: &mut W,
) -> CTResult<()> {
    let block = encode_block_size(ct_format);
    let mut buf = [0u8; STREAM_READ_BUF_SIZE];
    let mut pending = Vec::new();
    let mut line_col = 0usize;

    loop {
        let n = ct_input
            .read(&mut buf)
            .map_err(|_| CtSimpleError::new(1, "error: invalid input"))?;
        if n == 0 {
            break;
        }

        pending.extend_from_slice(&buf[..n]);
        let ready_len = pending.len() / block * block;
        if ready_len == 0 {
            continue;
        }

        let encoded = ctcore::ct_encoding::encode(ct_format, &pending[..ready_len]).map_err(
            |err| match err {
                CtEncodeError::InvalidInput => CtSimpleError::new(1, "error: invalid input"),
                _ => CtSimpleError::new(
                    1,
                    "error: invalid input (length must be multiple of 4 characters)",
                ),
            },
        )?;
        write_encoded_chunk(writer, encoded.as_bytes(), ct_line_wrap, &mut line_col)?;
        pending.drain(..ready_len);
    }

    if !pending.is_empty() {
        let encoded =
            ctcore::ct_encoding::encode(ct_format, &pending).map_err(|err| match err {
                CtEncodeError::InvalidInput => CtSimpleError::new(1, "error: invalid input"),
                _ => CtSimpleError::new(
                    1,
                    "error: invalid input (length must be multiple of 4 characters)",
                ),
            })?;
        write_encoded_chunk(writer, encoded.as_bytes(), ct_line_wrap, &mut line_col)?;
    }

    if ct_line_wrap > 0 && line_col > 0 {
        write_or_simple_error(writer, b"\n", false)?;
    }

    Ok(())
}

fn stream_decode_to_writer<R: Read, W: Write>(
    ct_input: &mut R,
    ct_format: Format,
    ct_ignore_garbage: bool,
    writer: &mut W,
) -> CTResult<()> {
    let block = decode_block_size(ct_format);
    let alphabet = decode_alphabet(ct_format);
    let mut buf = [0u8; STREAM_READ_BUF_SIZE];
    let mut pending = Vec::new();

    loop {
        let n = ct_input
            .read(&mut buf)
            .map_err(|_| CtSimpleError::new(1, "error: invalid input"))?;
        if n == 0 {
            break;
        }

        normalize_decode_input(&mut pending, &buf[..n], ct_ignore_garbage, alphabet);

        let ready_blocks = pending.len() / block;
        let decode_now = ready_blocks.saturating_sub(1) * block;
        if decode_now == 0 {
            continue;
        }

        let decoded = ctcore::ct_encoding::decode(ct_format, &pending[..decode_now])
            .map_err(|_| CtSimpleError::new(1, "error: invalid input"))?;
        write_or_simple_error(writer, &decoded, true)?;
        pending.drain(..decode_now);
    }

    if !pending.is_empty() {
        let decoded = ctcore::ct_encoding::decode(ct_format, &pending)
            .map_err(|_| CtSimpleError::new(1, "error: invalid input"))?;
        write_or_simple_error(writer, &decoded, true)?;
    }

    Ok(())
}

fn encode_block_size(format: Format) -> usize {
    match format {
        Format::Base64 | Format::Base64Url => 3,
        Format::Base32 | Format::Base32Hex => 5,
        Format::Base16 | Format::Base2Lsbf | Format::Base2Msbf => 1,
        // Base58 falls back to the whole-buffer path above.
        Format::Base58 => 1,
        Format::Z85 => 4,
    }
}

fn decode_block_size(format: Format) -> usize {
    match format {
        Format::Base64 | Format::Base64Url => 4,
        Format::Base32 | Format::Base32Hex => 8,
        Format::Base16 => 2,
        Format::Base2Lsbf | Format::Base2Msbf => 8,
        // Base58 falls back to the whole-buffer path above.
        Format::Base58 => 1,
        Format::Z85 => 5,
    }
}

fn decode_alphabet(format: Format) -> &'static [u8] {
    match format {
        Format::Base32 => b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567=",
        Format::Base64 => b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789=+/",
        Format::Base64Url => b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789=_-",
        Format::Base32Hex => b"0123456789ABCDEFGHIJKLMNOPQRSTUV=",
        Format::Base16 => b"0123456789ABCDEF",
        Format::Base2Lsbf | Format::Base2Msbf => b"01",
        Format::Base58 => b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz",
        Format::Z85 => {
            b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ.-:+=^!/*?&<>()[]{}@%$#"
        }
    }
}

fn normalize_decode_input(
    pending: &mut Vec<u8>,
    chunk: &[u8],
    ignore_garbage: bool,
    alphabet: &[u8],
) {
    if ignore_garbage {
        pending.extend(chunk.iter().copied().filter(|b| alphabet.contains(b)));
    } else {
        pending.extend(chunk.iter().copied().filter(|b| *b != b'\r' && *b != b'\n'));
    }
}

fn write_encoded_chunk<W: Write>(
    writer: &mut W,
    chunk: &[u8],
    wrap: usize,
    line_col: &mut usize,
) -> CTResult<()> {
    if wrap == 0 {
        return write_or_simple_error(writer, chunk, false);
    }

    let mut start = 0usize;
    while start < chunk.len() {
        let room = wrap - *line_col;
        let take = room.min(chunk.len() - start);
        write_or_simple_error(writer, &chunk[start..start + take], false)?;
        *line_col += take;
        start += take;
        if *line_col == wrap {
            write_or_simple_error(writer, b"\n", false)?;
            *line_col = 0;
        }
    }

    Ok(())
}

fn write_or_simple_error<W: Write>(
    writer: &mut W,
    bytes: &[u8],
    decode_path: bool,
) -> CTResult<()> {
    if writer.write_all(bytes).is_err() {
        if decode_path {
            return Err(CtSimpleError::new(1, "error: cannot write non-utf8 data"));
        }
        return Err(CtSimpleError::new(1, "error: invalid input"));
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::base_common;
    use ctcore::ct_encoding::Format;
    use std::ffi::OsString;
    use std::fs;
    use std::fs::File;
    use std::io::stdin;
    use std::io::{self, Cursor, Write};

    // Add the missing constants
    const BASE32_ABOUT: &str = "base32 encode or decode data";
    const BASE32_USAGE: &str = "base32 [OPTION]... [FILE]";

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
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), filename];
        let format = Format::Base16;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_encode_base32() {
        let filename = "base_common_Base32.txt";
        let content = "Test test_base_common_handle_input_encode_base32";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), filename];
        let format = Format::Base32;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }
    #[test]
    fn test_base_common_handle_input_encode_base32hex() {
        let filename = "base_common_Base32Hex.txt";
        let content = "Test test_base_common_handle_input_encode_base32hex";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), filename];
        let format = Format::Base32Hex;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_stream_encode_base58_matches_whole_buffer_encoding() {
        let input = b"hello base58";
        let mut reader = Cursor::new(input);
        let mut output = Vec::new();

        handle_base_input_streaming_to_writer(
            &mut reader,
            Format::Base58,
            Some(76),
            false,
            false,
            &mut output,
        )
        .expect("base58 streaming encode should succeed");

        let expected = ctcore::ct_encoding::encode(Format::Base58, input).unwrap();
        assert_eq!(String::from_utf8(output).unwrap().trim_end(), expected);
    }

    #[test]
    fn test_stream_decode_base58_matches_whole_buffer_decoding() {
        let encoded = ctcore::ct_encoding::encode(Format::Base58, b"hello base58").unwrap();
        let mut reader = Cursor::new(encoded.into_bytes());
        let mut output = Vec::new();

        handle_base_input_streaming_to_writer(
            &mut reader,
            Format::Base58,
            Some(76),
            false,
            true,
            &mut output,
        )
        .expect("base58 streaming decode should succeed");

        assert_eq!(output, b"hello base58");
    }

    #[test]
    fn test_base_common_handle_input_encode_base64() {
        let filename = "base_common_Base64.txt";
        let content = "Test test_base_common_handle_input_encode_base64";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), filename];
        let format = Format::Base64;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }
    #[test]
    fn test_base_common_handle_input_encode_base64url() {
        let filename = "base_common_Base64Url.txt";
        let content = "Test test_base_common_handle_input_encode_base64url";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), filename];
        let format = Format::Base64Url;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base16() {
        let filename = "base_common_decode_Base16.txt";
        let content = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "-d", filename];
        let format = Format::Base16;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base16_wrap() {
        let filename = "base_common_decode_Base16_wrap.txt";
        let content = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--decode", "--wrap=8", filename];
        let format = Format::Base16;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
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
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "-d", filename];
        let format = Format::Base32;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
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
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--decode", "--wrap=8", filename];
        let format = Format::Base32;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }
    #[test]
    fn test_base_common_handle_input_decode_base32hex() {
        let filename = "base_common_decode_Base32hex.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "-d", filename];
        let format = Format::Base32Hex;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base32hex_wrap() {
        let filename = "base_common_decode_Base32hex_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--decode", "--wrap=8", filename];
        let format = Format::Base32Hex;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base64() {
        let filename = "base_common_decode_Base64.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64";
        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "-d", filename];
        let format = Format::Base64;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base64_wrap() {
        let filename = "base_common_decode_Base64_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64";
        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--decode", "--wrap=8", filename];
        let format = Format::Base64;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base64url() {
        let filename = "base_common_decode_Base64Url.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64url";

        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "-d", filename];
        let format = Format::Base64Url;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base64url_wrap() {
        let filename = "base_common_decode_Base64Url_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64url";

        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--decode", "--wrap=6", filename];
        let format = Format::Base64Url;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                println!("{expected_output}");
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_encode_base2lsbf() {
        let filename = "base_common_encode_Base2Lsbf.txt";
        let content = "Test Base2Lsbf";

        let expected_output = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), filename];
        let format = Format::Base2Lsbf;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_encode_base2msbf() {
        let filename = "base_common_encode_Base2Msbf.txt";
        let content = "Test Base2Msbf";

        let expected_output = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), filename];
        let format = Format::Base2Msbf;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_encode_base2lsbf_wrap() {
        let filename = "base_common_encode_Base2Lsbf_wrap.txt";
        let content = "Test Base2Lsbf";

        let expected_output = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--wrap=8", filename];
        let format = Format::Base2Lsbf;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_encode_base2msbf_wrap() {
        let filename = "base_common_encode_Base2Msbf_wrap.txt";
        let content = "Test Base2Msbf";

        let expected_output = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--wrap=8", filename];
        let format = Format::Base2Msbf;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base2lsbf() {
        let filename = "base_common_decode_Base2Lsbf.txt";
        let expected_output = "Test Base2Lsbf";

        let content = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--decode", filename];
        let format = Format::Base2Lsbf;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base2msbf() {
        let filename = "base_common_decode_Base2Msbf.txt";
        let expected_output = "Test Base2Msbf";

        let content = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--decode", filename];
        let format = Format::Base2Msbf;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base2lsbf_wrap() {
        let filename = "base_common_decode_Base2Lsbf_wrap.txt";
        let expected_output = "Test Base2Lsbf";

        let content = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--decode", "--wrap=6", filename];
        let format = Format::Base2Lsbf;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_base2msbf_wrap() {
        let filename = "base_common_decode_Base2Msbf_wrap.txt";
        let expected_output = "Test Base2Msbf";

        let content = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--decode", "--wrap=6", filename];
        let format = Format::Base2Msbf;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_encode_basez85() {
        let filename = "base_common_encode_Base2z85.txt";
        let content = "TestBZ85";

        let expected_output = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), filename];
        let format = Format::Z85;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_encode_basez85_wrap() {
        let filename = "test_base_common_handle_input_encode_basez85_wrap.txt";
        let content = "TestBZ85";

        let expected_output = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--wrap=8", filename];
        let format = Format::Z85;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_basez85() {
        let filename = "test_base_common_handle_input_decode_basez85.txt";
        let expected_output = "TestBZ85";

        let content = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--decode", filename];
        let format = Format::Z85;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                println!("result:{s}");
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_base_common_handle_input_decode_basez85_wrap() {
        let filename = "test_base_common_handle_input_decode_basez85_wrap.txt";
        let expected_output = "TestBZ85";

        let content = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--decode", "--wrap=8", filename];
        let format = Format::Z85;
        let config: base_common::BaseConfig = base_common::base_parsing_command_args(
            args.iter().map(OsString::from),
            BASE32_ABOUT.to_string(),
            BASE32_USAGE.to_string(),
        )
        .expect("parse_base_cmd_args Failed");

        let stdin_raw = stdin();
        let mut input: Box<dyn Read> =
            base_common::get_base_input(&config, &stdin_raw).expect("get_input Failed");

        let mut output = Vec::new();
        let result = base_common::handle_base_input(
            &mut input,
            &mut output,
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
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
                // println!("result:{}", s);
                // println!("{}", expected_output);
            }
        }
        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    fn core_output_bytes(
        input: &[u8],
        format: Format,
        wrap: Option<usize>,
        ignore_garbage: bool,
        decode: bool,
    ) -> Result<Vec<u8>, String> {
        let mut reader = Cursor::new(input.to_vec());
        base_common::handle_base_input_core(&mut reader, format, wrap, ignore_garbage, decode)
            .map(|out| out.bytes)
            .map_err(|e| e.to_string())
    }

    fn streaming_output_bytes(
        input: &[u8],
        format: Format,
        wrap: Option<usize>,
        ignore_garbage: bool,
        decode: bool,
    ) -> Result<Vec<u8>, String> {
        let mut reader = Cursor::new(input.to_vec());
        let mut out = Vec::new();
        base_common::handle_base_input_streaming_to_writer(
            &mut reader,
            format,
            wrap,
            ignore_garbage,
            decode,
            &mut out,
        )
        .map(|_| out)
        .map_err(|e| e.to_string())
    }

    fn assert_streaming_matches_core(
        input: &[u8],
        format: Format,
        wrap: Option<usize>,
        ignore_garbage: bool,
        decode: bool,
    ) {
        let core = core_output_bytes(input, format, wrap, ignore_garbage, decode);
        let streaming = streaming_output_bytes(input, format, wrap, ignore_garbage, decode);
        assert_eq!(streaming, core);
    }

    #[test]
    fn test_streaming_encode_matches_core_base64_default_wrap() {
        let mut input = Vec::with_capacity(STREAM_READ_BUF_SIZE + 137);
        for i in 0..(STREAM_READ_BUF_SIZE + 137) {
            input.push((i % 251) as u8);
        }

        assert_streaming_matches_core(&input, Format::Base64, None, false, false);
    }

    #[test]
    fn test_streaming_decode_matches_core_base64_ignore_garbage() {
        let raw = b"streaming parity for base64 decode with ignore garbage".repeat(256);
        let encoded = ctcore::ct_encoding::encode(Format::Base64, &raw).unwrap();

        let mut noisy = String::with_capacity(encoded.len() + encoded.len() / 8);
        for (idx, ch) in encoded.chars().enumerate() {
            noisy.push(ch);
            if idx % 11 == 0 {
                noisy.push(' ');
                noisy.push('\n');
                noisy.push('#');
            }
        }

        assert_streaming_matches_core(noisy.as_bytes(), Format::Base64, None, true, true);
    }

    #[test]
    fn test_streaming_decode_matches_core_base16_ignore_garbage() {
        let raw = vec![0xde, 0xad, 0xbe, 0xef, 0x12, 0x34, 0xab, 0xcd];
        let encoded = ctcore::ct_encoding::encode(Format::Base16, &raw).unwrap();

        let noisy = format!(
            "{}--{}  \n{}",
            &encoded[0..4],
            &encoded[4..8],
            &encoded[8..]
        );
        assert_streaming_matches_core(noisy.as_bytes(), Format::Base16, None, true, true);
    }
}
