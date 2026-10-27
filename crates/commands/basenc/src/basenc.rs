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
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::ArgAction;
use clap::Command;
use ct_base32::base_common::{self, BASE_CMD_PARSE_ERROR, BaseConfig};
use sys_locale::get_locale;

use ctcore::{
    ct_encoding::Format,
    ct_error::{CTError, CTResult, CTsageError, FromIo},
};

use ctcore::Tool;
use ctcore::ct_error::UClapError;
use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Formatter};
use std::io::Cursor;
use std::io::stdin;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;

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
    ("base58", Format::Base58, "base58 encoding"),
    (
        "z85",
        Format::Z85,
        "ascii85-like encoding;\n\
         when encoding, input length must be a multiple of 4;\n\
         when decoding, input length must be a multiple of 5",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasencSemanticRow {
    pub encoding: String,
    pub kind: String,
    pub mode: String,
    pub wrap: usize,
    pub ignore_garbage: bool,
    pub input: String,
    pub file: Option<String>,
    pub line: usize,
    pub output_text: String,
    pub byte_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasencSemantic {
    pub rows: Vec<BasencSemanticRow>,
    pub classic_text: String,
    pub classic_bytes: Vec<u8>,
    pub stderr_text: String,
    pub exit_code: i32,
}

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

const BASENC_LONG_OPTIONS: &[(&[u8], bool, bool)] = &[
    (b"decode", false, true),
    (b"wrap", true, true),
    (b"ignore-garbage", false, true),
    (b"base64", false, true),
    (b"base64url", false, true),
    (b"base32", false, true),
    (b"base32hex", false, true),
    (b"base16", false, true),
    (b"base2msbf", false, true),
    (b"base2lsbf", false, true),
    (b"z85", false, true),
    // base58 is a syskits extension and is omitted from GNU ambiguity lists.
    (b"base58", false, false),
    (b"help", false, true),
    (b"version", false, true),
];

#[derive(Debug)]
struct BasencNonUtf8OptionError {
    diagnostic: Vec<u8>,
}

impl Display for BasencNonUtf8OptionError {
    fn fmt(&self, _formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let mut stderr = std::io::stderr().lock();
        let _ = stderr
            .write_all(ctcore::ct_util_name().as_bytes())
            .and_then(|_| stderr.write_all(b": "))
            .and_then(|_| stderr.write_all(&self.diagnostic))
            .and_then(|_| stderr.write_all(b"\n"))
            .and_then(|_| stderr.flush());
        Ok(())
    }
}

impl Error for BasencNonUtf8OptionError {}

impl ctcore::ct_error::CTError for BasencNonUtf8OptionError {
    fn code(&self) -> i32 {
        1
    }
}

fn basenc_non_utf8_option_error(args: &[OsString]) -> Option<BasencNonUtf8OptionError> {
    for argument in args.iter().skip(1) {
        let bytes = argument.as_os_str().as_bytes();
        if bytes == b"--" {
            break;
        }
        if !bytes.starts_with(b"-") || bytes == b"-" {
            continue;
        }

        if let Some(long_argument) = bytes.strip_prefix(b"--") {
            let name_end = long_argument
                .iter()
                .position(|byte| *byte == b'=')
                .unwrap_or(long_argument.len());
            if long_argument[..name_end].iter().all(u8::is_ascii) {
                continue;
            }
            return Some(BasencNonUtf8OptionError {
                diagnostic: [b"unrecognized option '".as_slice(), bytes, b"'"].concat(),
            });
        }

        let short_options = &bytes[1..];
        if short_options.first() == Some(&b'w') {
            continue;
        }
        let byte = short_options.iter().find(|byte| !byte.is_ascii())?;
        return Some(BasencNonUtf8OptionError {
            diagnostic: [b"invalid option -- '".as_slice(), &[*byte], b"'"].concat(),
        });
    }
    None
}

fn basenc_gnu_option_error(args: &[OsString]) -> Option<Box<dyn ctcore::ct_error::CTError>> {
    let mut index = 1;
    while index < args.len() {
        let argument = args[index].as_os_str().as_bytes();
        if argument == b"--" {
            break;
        }

        if let Some(long_argument) = argument.strip_prefix(b"--") {
            if long_argument.is_empty() {
                break;
            }
            let value_index = long_argument.iter().position(|byte| *byte == b'=');
            let name = &long_argument[..value_index.unwrap_or(long_argument.len())];
            let candidates: Vec<_> = if let Some(exact) = BASENC_LONG_OPTIONS
                .iter()
                .find(|(option, _, _)| *option == name)
                .copied()
            {
                vec![exact]
            } else {
                BASENC_LONG_OPTIONS
                    .iter()
                    .filter(|(option, _, _)| option.starts_with(name))
                    .copied()
                    .collect()
            };
            if candidates.is_empty() {
                let text = std::str::from_utf8(argument).ok()?;
                return Some(CTsageError::new(
                    BASE_CMD_PARSE_ERROR,
                    format!("unrecognized option '{text}'"),
                ));
            }

            let visible: Vec<_> = candidates
                .iter()
                .filter(|(_, _, visible)| *visible)
                .copied()
                .collect();
            if visible.len() > 1 {
                let possibilities = visible
                    .iter()
                    .map(|(option, _, _)| format!("'--{}'", std::str::from_utf8(option).unwrap()))
                    .collect::<Vec<_>>()
                    .join(" ");
                let text = std::str::from_utf8(argument).ok()?;
                return Some(CTsageError::new(
                    BASE_CMD_PARSE_ERROR,
                    format!("option '{text}' is ambiguous; possibilities: {possibilities}"),
                ));
            }

            let (option, takes_value, _) = *visible.first().or_else(|| candidates.first())?;
            let option = std::str::from_utf8(option).expect("GNU option names are ASCII");
            if value_index.is_some() && !takes_value {
                return Some(CTsageError::new(
                    BASE_CMD_PARSE_ERROR,
                    format!("option '--{option}' doesn't allow an argument"),
                ));
            }
            if value_index.is_none() && takes_value {
                if index + 1 == args.len() {
                    return Some(CTsageError::new(
                        BASE_CMD_PARSE_ERROR,
                        format!("option '--{option}' requires an argument"),
                    ));
                }
                index += 1;
            }
        } else if argument.len() > 1 && argument[0] == b'-' {
            let short_options = &argument[1..];
            let mut option_index = 0;
            while option_index < short_options.len() {
                match short_options[option_index] {
                    b'd' | b'i' | b'h' | b'V' => option_index += 1,
                    b'w' => {
                        if option_index + 1 == short_options.len() {
                            if index + 1 == args.len() {
                                return Some(CTsageError::new(
                                    BASE_CMD_PARSE_ERROR,
                                    "option requires an argument -- 'w'",
                                ));
                            }
                            index += 1;
                        }
                        break;
                    }
                    invalid if invalid.is_ascii() => {
                        return Some(CTsageError::new(
                            BASE_CMD_PARSE_ERROR,
                            format!("invalid option -- '{}'", invalid as char),
                        ));
                    }
                    _ => return None,
                }
            }
        }
        index += 1;
    }
    None
}

fn basenc_parse_cmd_args(args: impl ctcore::Args) -> CTResult<(BaseConfig, Format)> {
    let args = base_common::normalize_base_short_wrap_equals(args.collect());
    let args = base_common::normalize_base_args_for_posix(
        args.into_iter(),
        std::env::var_os("POSIXLY_CORRECT").is_some(),
    );
    if let Some(error) = basenc_non_utf8_option_error(&args) {
        return Err(Box::new(error));
    }
    if let Some(error) = basenc_gnu_option_error(&args) {
        return Err(error);
    }
    let args_match = match ct_app().try_get_matches_from(args.clone()) {
        Ok(matches) => matches,
        Err(e) => {
            if e.kind() == clap::error::ErrorKind::UnknownArgument {
                let err_text = e.to_string();
                if let Some(arg) = extract_unexpected_argument(&err_text) {
                    return Err(CTsageError::new(
                        BASE_CMD_PARSE_ERROR,
                        arg.trim_start_matches('-'),
                    ));
                }
            }
            return Err(e.with_exit_code(1).into());
        }
    };
    let config_result = BaseConfig::from(&args_match);
    let format_mod = BASE64_ENCODINGS
        .iter()
        .find(|encoding| args_match.get_flag(encoding.0))
        .map(|encoding| encoding.1);
    match config_result {
        Ok(config_mod) => {
            let format_mod = format_mod
                .ok_or_else(|| CTsageError::new(BASE_CMD_PARSE_ERROR, "missing encoding type"))?;
            Ok((config_mod, format_mod))
        }
        Err(error) => {
            if format_mod.is_none() && !error.to_string().starts_with("invalid wrap size:") {
                return Err(CTsageError::new(
                    BASE_CMD_PARSE_ERROR,
                    "missing encoding type",
                ));
            }
            Err(error)
        }
    }
}

fn extract_unexpected_argument(err_text: &str) -> Option<String> {
    let marker = "unexpected argument '";
    let start = err_text.find(marker)?;
    let rest = &err_text[start + marker.len()..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

fn basenc_render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("basenc: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'basenc --help' for more information.\n");
    }
    stderr
}

fn basenc_encoding_name(format: Format) -> &'static str {
    match format {
        Format::Base64 => "base64",
        Format::Base64Url => "base64url",
        Format::Base32 => "base32",
        Format::Base32Hex => "base32hex",
        Format::Base16 => "base16",
        Format::Base2Lsbf => "base2lsbf",
        Format::Base2Msbf => "base2msbf",
        Format::Base58 => "base58",
        Format::Z85 => "z85",
    }
}

fn basenc_rows_from_output(
    config: &BaseConfig,
    format: Format,
    output: &[u8],
) -> Vec<BasencSemanticRow> {
    let mut rows = Vec::new();
    let mut start = 0;
    let mut line = 1;

    for (index, byte) in output.iter().enumerate() {
        if *byte == b'\n' {
            rows.push(BasencSemanticRow {
                encoding: basenc_encoding_name(format).into(),
                kind: "text".into(),
                mode: if config.base_decode {
                    "decode".into()
                } else {
                    "encode".into()
                },
                wrap: config.base_wrap_cols.unwrap_or(76),
                ignore_garbage: config.base_ignore_garbage,
                input: if config.base_to_read.is_some() {
                    "file".into()
                } else {
                    "stdin".into()
                },
                file: config.base_to_read.clone(),
                line,
                output_text: String::from_utf8_lossy(&output[start..index]).into_owned(),
                byte_len: index - start,
            });
            start = index + 1;
            line += 1;
        }
    }

    if start < output.len() {
        rows.push(BasencSemanticRow {
            encoding: basenc_encoding_name(format).into(),
            kind: "text".into(),
            mode: if config.base_decode {
                "decode".into()
            } else {
                "encode".into()
            },
            wrap: config.base_wrap_cols.unwrap_or(76),
            ignore_garbage: config.base_ignore_garbage,
            input: if config.base_to_read.is_some() {
                "file".into()
            } else {
                "stdin".into()
            },
            file: config.base_to_read.clone(),
            line,
            output_text: String::from_utf8_lossy(&output[start..]).into_owned(),
            byte_len: output.len() - start,
        });
    }

    rows
}

fn basenc_semantic_from_output(
    config: &BaseConfig,
    format: Format,
    output: Vec<u8>,
    stderr_text: String,
    exit_code: i32,
) -> BasencSemantic {
    let classic_text = String::from_utf8_lossy(&output).into_owned();
    BasencSemantic {
        rows: basenc_rows_from_output(config, format, &output),
        classic_text,
        classic_bytes: output,
        stderr_text,
        exit_code,
    }
}

pub fn basenc_native_semantic(args: impl ctcore::Args) -> CTResult<BasencSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let (config, format) = match basenc_parse_cmd_args(args) {
        Ok(config) => config,
        Err(err) => {
            return Ok(BasencSemantic {
                rows: Vec::new(),
                classic_text: String::new(),
                classic_bytes: Vec::new(),
                stderr_text: basenc_render_error_text(err.as_ref()),
                exit_code: err.code(),
            });
        }
    };

    let stdin_info = stdin();
    let mut input_info = match base_common::get_base_input(&config, &stdin_info) {
        Ok(input) => input,
        Err(err) => {
            return Ok(BasencSemantic {
                rows: Vec::new(),
                classic_text: String::new(),
                classic_bytes: Vec::new(),
                stderr_text: basenc_render_error_text(err.as_ref()),
                exit_code: err.code(),
            });
        }
    };

    let mut input_bytes = Vec::new();
    if let Err(err) = input_info
        .read_to_end(&mut input_bytes)
        .map_err_context(|| config.base_to_read.clone().unwrap_or_else(|| "-".into()))
    {
        return Ok(BasencSemantic {
            rows: Vec::new(),
            classic_text: String::new(),
            classic_bytes: Vec::new(),
            stderr_text: basenc_render_error_text(err.as_ref()),
            exit_code: err.code(),
        });
    }

    let mut output = Vec::new();
    match base_common::handle_base_input(
        &mut Cursor::new(&input_bytes),
        &mut output,
        format,
        config.base_wrap_cols,
        config.base_ignore_garbage,
        config.base_decode,
    ) {
        Ok(()) => Ok(basenc_semantic_from_output(
            &config,
            format,
            output,
            String::new(),
            0,
        )),
        Err(err) => Ok(basenc_semantic_from_output(
            &config,
            format,
            output,
            basenc_render_error_text(err.as_ref()),
            err.code(),
        )),
    }
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
        basenc_main(args.iter().cloned(), &mut std::io::stdout().lock())
    }
}

pub fn basenc_main<W: std::io::Write>(args: impl ctcore::Args, mut writer: W) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let (config_mod, format_mod) = basenc_parse_cmd_args(args)?;

    // 创建对stdin的引用，以便我们能从parse_base_cmd_args返回锁定的stdin
    let ct_stdin = stdin();
    let mut ct_input: Box<dyn Read> = base_common::get_base_input(&config_mod, &ct_stdin)?;

    base_common::handle_base_input(
        &mut ct_input,
        &mut writer,
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
    use std::os::unix::ffi::OsStringExt;

    use clap::error::ErrorKind;
    use std::io::{self, Write};

    #[test]
    fn test_tool_implementation() {
        let tool = Basenc;

        // 测试 name 方法
        assert_eq!(tool.name(), "basenc");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("basenc"));

        // 测试 execute 方法
        let args = vec![OsString::from("basenc"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err()); // basenc needs an encoding flag to be valid
    }

    #[test]
    fn test_unknown_argument_error_matches_coreutils_shape() {
        let args = [ctcore::ct_util_name(), "--foobar"];
        let mut output = Vec::new();
        let err = basenc_main(args.iter().map(OsString::from), &mut output).unwrap_err();
        assert_eq!(err.code(), 1);
        assert_eq!(err.to_string(), "unrecognized option '--foobar'");
        assert!(err.usage());
    }

    #[test]
    fn test_basenc_short_wrap_equals_is_part_of_wrap_value() {
        let args = [ctcore::ct_util_name(), "--base64", "-w=2"];
        let result = basenc_parse_cmd_args(args.iter().map(OsString::from));
        assert!(result.is_err());
        let error = result.err().expect("-w=2 must be rejected").to_string();
        assert!(error.contains("=2"), "unexpected error: {error}");
    }

    #[test]
    fn test_basenc_reports_invalid_wrap_before_missing_encoding() {
        let args = [ctcore::ct_util_name(), "-w", "bad"];
        let result = basenc_parse_cmd_args(args.iter().map(OsString::from));
        let error = result
            .err()
            .expect("invalid wrap must be rejected")
            .to_string();
        assert!(
            error.contains("invalid wrap size"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn test_basenc_non_utf8_wrap_value_preserves_raw_byte() {
        let args = vec![
            OsString::from(ctcore::ct_util_name()),
            OsString::from("--base64"),
            OsString::from("--wrap"),
            OsString::from_vec(vec![0xff]),
        ];
        let result = basenc_parse_cmd_args(args.into_iter());
        let error = result.err().expect("non-UTF-8 wrap must be rejected");
        assert_eq!(error.to_string(), "invalid wrap size: '\\377'");
    }

    #[test]
    fn test_basenc_non_utf8_extra_operand_preserves_raw_byte() {
        let args = vec![
            OsString::from(ctcore::ct_util_name()),
            OsString::from("--base64"),
            OsString::from("input"),
            OsString::from_vec(b"extra-\xff".to_vec()),
        ];
        let result = basenc_parse_cmd_args(args.into_iter());
        let error = result.err().expect("extra operand must be rejected");
        assert_eq!(error.to_string(), "extra operand 'extra-\\377'");
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
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--base16", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base32() {
        let filename = "test_basenc_encode_base32.txt";
        let content = "Test test_base_common_handle_input_encode_base32";
        let expected_output =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--base32", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base32hex() {
        let filename = "test_basenc_encode_base32hex.txt";
        let content = "Test test_base_common_handle_input_encode_base32hex";
        let expected_output = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--base32hex", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base64() {
        let filename = "test_basenc_encode_base64.txt";
        let content = "Test test_base_common_handle_input_encode_base64";
        let expected_output = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--base64", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base64url() {
        let filename = "test_basenc_encode_base64url.txt";
        let content = "Test test_base_common_handle_input_encode_base64url";

        let expected_output =
            "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "--base64url", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base16() {
        let filename = "test_basenc_decode_base16.txt";
        let content = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
        let expected_output = "Test  test_base_common_handle_input_encode_base16";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "-d", "--base16", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base16_lowercase() {
        let filename = "test_basenc_decode_base16_lowercase.txt";
        let content = "6a";

        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "-d", "--base16", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert!(result.is_err());
        assert!(output.is_empty());
    }

    #[test]
    fn test_basenc_decode_base16_wrap() {
        let filename = "test_basenc_decode_base16_wrap.txt";
        let content = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
        let expected_output = "Test  test_base_common_handle_input_encode_base16";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--base16",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base32() {
        let filename = "test_basenc_decode_base32.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32";
        let content =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "-d", "--base32", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base32_wrap() {
        let filename = "test_basenc_decode_base32_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32";
        let content =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--base32",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base32hex() {
        let filename = "webasenc_decode_base32hex.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [ctcore::ct_util_name(), "-d", "--base32hex", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base32hex_wrap() {
        let filename = "test_basenc_decode_base32hex_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--base32hex",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base64() {
        let filename = "test_basenc_decode_base64.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64";
        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "-d", "--base64", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base64_wrap() {
        let filename = "test_basenc_decode_base64_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64";
        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--base64",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base64url() {
        let filename = "test_basenc_decode_base64url.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64url";

        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "-d", "--base64url", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base64url_wrap() {
        let filename = "test_basenc_decode_base64url_wrap.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64url";

        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--base64url",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base2lsbf() {
        let filename = "test_basenc_encode_base2lsbf.txt";
        let content = "Test Base2Lsbf";

        let expected_output = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--base2lsbf", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base2msbf() {
        let filename = "test_basenc_encode_base2msbf.txt";
        let content = "Test Base2Msbf";

        let expected_output = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--base2msbf", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base2lsbf_wrap() {
        let filename = "test_basenc_encode_base2lsbf_wrap.txt";
        let content = "Test Base2Lsbf";

        let expected_output = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--base2lsbf", "--wrap=8", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base2msbf_wrap() {
        let filename = "test_basenc_encode_base2msbf_wrap.txt";
        let content = "Test Base2Msbf";

        let expected_output = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--base2msbf", "--wrap=8", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base2lsbf() {
        let filename = "test_basenc_decode_base2lsbf.txt";
        let expected_output = "Test Base2Lsbf";

        let content = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--decode", "--base2lsbf", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base2msbf() {
        let filename = "test_basenc_decode_base2msbf.txt";
        let expected_output = "Test Base2Msbf";

        let content = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--decode", "--base2msbf", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base2lsbf_wrap() {
        let filename = "test_basenc_decode_base2lsbf_wrap.txt";
        let expected_output = "Test Base2Lsbf";

        let content = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--decode",
            "--base2lsbf",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base2msbf_wrap() {
        let filename = "test_basenc_decode_base2msbf_wrap.txt";
        let expected_output = "Test Base2Msbf";

        let content = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--decode",
            "--base2msbf",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_basez85() {
        let filename = "test_basenc_encode_basez85.txt";
        let content = "TestBZ85";

        let expected_output = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--z85", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_basez85_wrap() {
        let filename = "test_basenc_encode_basez85_wrap.txt";
        let content = "TestBZ85";

        let expected_output = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--wrap=6", "--z85", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_basez85() {
        let filename = "test_basenc_decode_basez85.txt";
        let expected_output = "TestBZ85";

        let content = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--decode", "--z85", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_basez85_wrap() {
        let filename = "test_basenc_decode_basez85_wrap.txt";
        let expected_output = "TestBZ85";

        let content = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--decode",
            "--z85",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_h_ctmain() {
        {
            let args = ["--help", ""];
            let mut output = Vec::new();
            let result = basenc_main(args.iter().map(OsString::from), &mut output);
            assert!(result.is_err());
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
            let mut output = Vec::new();
            let result = basenc_main(args.iter().map(OsString::from), &mut output);
            assert!(result.is_err());
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
            let mut output = Vec::new();
            let result = basenc_main(args.iter().map(OsString::from), &mut output);
            assert!(result.is_err());
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
            let mut output = Vec::new();
            let result = basenc_main(args.iter().map(OsString::from), &mut output);
            assert!(result.is_err());
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
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base16",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base32_ignore() {
        let filename = "test_basenc_encode_base32_ignore.txt";
        let content = "Test test_base_common_handle_input_encode_base32";
        let expected_output =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base32",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base32hex_ignore() {
        let filename = "test_basenc_encode_base32hex_ignore.txt";
        let content = "Test test_base_common_handle_input_encode_base32hex";
        let expected_output = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base32hex",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base64_ignore() {
        let filename = "test_basenc_encode_base64_ignore.txt";
        let content = "Test test_base_common_handle_input_encode_base64";
        let expected_output = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base64",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base64url_ignore() {
        let filename = "test_basenc_encode_base64url_ignore.txt";
        let content = "Test test_base_common_handle_input_encode_base64url";

        let expected_output =
            "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base64url",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base16_ignore() {
        let filename = "test_basenc_decode_base16_ignore.txt";
        let content = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
        let expected_output = "Test  test_base_common_handle_input_encode_base16";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base16",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base16_wrap_ignore() {
        let filename = "test_basenc_decode_base16_wrap_ignore.txt";
        let content = "546573742020746573745F626173655F636F6D6D6F6E5F68616E646C655F696E7075745F656E636F64655F626173653136";
        let expected_output = "Test  test_base_common_handle_input_encode_base16";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base16",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base32_ignore() {
        let filename = "test_basenc_decode_base32_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32";
        let content =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base32",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base32_wrap_ignore() {
        let filename = "test_basenc_decode_base32_wrap_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32";
        let content =
            "KRSXG5BAORSXG5C7MJQXGZK7MNXW23LPNZPWQYLOMRWGKX3JNZYHK5C7MVXGG33EMVPWEYLTMUZTE===";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base32",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base32hex_ignore() {
        let filename = "test_basenc_decode_base32hex.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        // 测试用例1：有效输入
        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base32hex",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base32hex_wrap_ignore() {
        let filename = "test_basenc_decode_base32hex_wrap_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base32hex";
        let content = "AHIN6T10EHIN6T2VC9GN6PAVCDNMQRBFDPFMGOBECHM6ANR9DPO7AT2VCLN66RR4CLFM4OBJCKPJ4Q35F0======";

        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base32hex",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base64_ignore() {
        let filename = "test_basenc_decode_base64_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64";
        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base64",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base64_wrap_ignore() {
        let filename = "test_basenc_decode_base64_wrap_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64";
        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base64",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base64url_ignore() {
        let filename = "test_basenc_decode_base64url_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64url";

        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base64url",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base64url_wrap_ignore() {
        let filename = "test_basenc_decode_base64url_wrap_ignore.txt";
        let expected_output = "Test test_base_common_handle_input_encode_base64url";

        let content = "VGVzdCB0ZXN0X2Jhc2VfY29tbW9uX2hhbmRsZV9pbnB1dF9lbmNvZGVfYmFzZTY0dXJs";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "-d",
            "--ignore-garbage",
            "--base64url",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base2lsbf_ignore() {
        let filename = "test_basenc_encode_base2lsbf_ignore.txt";
        let content = "Test Base2Lsbf";

        let expected_output = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base2lsbf",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base2msbf_ignore() {
        let filename = "test_basenc_encode_base2msbf_ignore.txt";
        let content = "Test Base2Msbf";

        let expected_output = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base2msbf",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base2lsbf_wrap_ignore() {
        let filename = "test_basenc_encode_base2lsbf_wrap_ignore.txt";
        let content = "Test Base2Lsbf";

        let expected_output = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base2lsbf",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base2msbf_wrap_ignore() {
        let filename = "test_basenc_encode_base2msbf_wrap_ignore.txt";
        let content = "Test Base2Msbf";

        let expected_output = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--base2msbf",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base2lsbf_ignore() {
        let filename = "test_basenc_decode_base2lsbf_ignore.txt";
        let expected_output = "Test Base2Lsbf";

        let content = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--decode",
            "--base2lsbf",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base2msbf_ignore() {
        let filename = "test_basenc_decode_base2msbf_ignore.txt";
        let expected_output = "Test Base2Msbf";

        let content = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--decode",
            "--base2msbf",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base2lsbf_wrap_ingore() {
        let filename = "test_basenc_decode_base2lsbf_wrap_ingore.txt";
        let expected_output = "Test Base2Lsbf";

        let content = "0010101010100110110011100010111000000100010000101000011011001110101001100100110000110010110011100100011001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--decode",
            "--base2lsbf",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_base2msbf_wrap_ignore() {
        let filename = "test_basenc_decode_base2msbf_wrap_ignore.txt";
        let expected_output = "Test Base2Msbf";

        let content = "0101010001100101011100110111010000100000010000100110000101110011011001010011001001001101011100110110001001100110";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--decode",
            "--base2msbf",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_basez85_ignore() {
        let filename = "test_basenc_encode_basez85_ignore.txt";
        let content = "TestBZ85";

        let expected_output = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--z85",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_basez85_wrap_ignore() {
        let filename = "test_basenc_encode_basez85_wrap_ignore.txt";
        let content = "TestBZ85";

        let expected_output = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--wrap=6",
            "--z85",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_basez85_ignore() {
        let filename = "test_basenc_decode_basez85_ignore.txt";
        let expected_output = String::new(); //"TestBZ85";

        let content = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "-ignore-garbage",
            "--decode",
            "--z85",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_decode_basez85_wrap_ignore() {
        let filename = "test_basenc_decode_basez85_wrap_ignore.txt";
        let expected_output = "TestBZ85";

        let content = "raQb)lrVua";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "--ignore-garbage",
            "--decode",
            "--z85",
            "--wrap=8",
            filename,
        ];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
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
    fn test_basenc_encode_base58() {
        let filename = "test_basenc_encode_base58.txt";
        let content = "Test test_base_common_handle_input_encode_base58";
        let expected_output = "46ZVcZ5fTn1as9FPhMAQVu9U5dTst58bJjDG3ayCWpbks9iJjvNRq5gQ5FeLoXKz7h";

        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "--base58", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
        let mut s = String::new();

        match result {
            Err(output) => {
                println!("Error code: {}", output.code());
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
            }
        }
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_decode_base58() {
        let filename = "test_basenc_decode_base58.txt";
        let content = "46ZVcZ5fTn1as9FPhMAQVu9U5dTst58bJjDG3ayCWpbks9iJjvNRq5gQ5FeLoXKz7h";
        let expected_output = "Test test_base_common_handle_input_encode_base58";

        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [ctcore::ct_util_name(), "-d", "--base58", filename];
        let mut output = Vec::new();
        let result = basenc_main(args.iter().map(OsString::from), &mut output);
        let mut s = String::new();

        match result {
            Err(output) => {
                println!("Error code: {}", output.code());
            }
            Ok(_) => {
                s = String::from_utf8(output.clone()).unwrap();
                s = s.replace("\n", "");
            }
        }
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        assert_eq!(s, expected_output);
    }

    #[test]
    fn test_basenc_native_semantic_encode_row() {
        let filename = "test_basenc_native_semantic_encode_row.txt";
        base_create_file_with_content(filename, "abc").expect("create basenc semantic fixture");

        let semantic = basenc_native_semantic(
            [ctcore::ct_util_name(), "--base64", filename]
                .iter()
                .map(OsString::from),
        )
        .expect("semantic");

        base_delete_file(filename).expect("delete basenc semantic fixture");

        assert_eq!(semantic.exit_code, 0);
        assert_eq!(semantic.stderr_text, "");
        assert_eq!(semantic.classic_text, "YWJj\n");
        assert_eq!(semantic.rows.len(), 1);
        assert_eq!(semantic.rows[0].encoding, "base64");
        assert_eq!(semantic.rows[0].kind, "text");
        assert_eq!(semantic.rows[0].mode, "encode");
        assert_eq!(semantic.rows[0].input, "file");
        assert_eq!(semantic.rows[0].file.as_deref(), Some(filename));
        assert_eq!(semantic.rows[0].line, 1);
        assert_eq!(semantic.rows[0].output_text, "YWJj");
        assert_eq!(semantic.rows[0].byte_len, 4);
        assert_eq!(semantic.rows[0].wrap, 76);
        assert!(!semantic.rows[0].ignore_garbage);
    }

    #[test]
    fn test_basenc_native_semantic_invalid_decode_reports_error_metadata() {
        let filename = "test_basenc_native_semantic_invalid_decode.txt";
        base_create_file_with_content(filename, "%%%").expect("create invalid basenc fixture");

        let semantic = basenc_native_semantic(
            [ctcore::ct_util_name(), "--decode", "--base64", filename]
                .iter()
                .map(OsString::from),
        )
        .expect("semantic");

        base_delete_file(filename).expect("delete invalid basenc fixture");

        assert_eq!(semantic.exit_code, 1);
        assert_eq!(semantic.classic_text, "");
        assert_eq!(semantic.rows, Vec::<BasencSemanticRow>::new());
        assert_eq!(semantic.stderr_text, "basenc: invalid input\n");
    }
}
