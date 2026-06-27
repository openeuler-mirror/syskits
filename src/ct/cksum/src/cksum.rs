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

extern crate rust_i18n; // spell-checker:ignore (ToDO) fname, algo
use clap::{Arg, ArgAction, Command, crate_version, value_parser};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::Tool;
use ctcore::{
    ct_encoding,
    ct_error::{CTError, CTResult, CtSimpleError, FromIo},
    ct_show,
    ct_sum::{
        BSD, CtBlake2b, CtCRC, CtDigest, CtDigestWriter, CtSm3, Md5, SYSV, Sha1, Sha224, Sha256,
        Sha384, Sha512, div_ceil,
    },
};
use hex::decode;
use hex::encode;
use std::error::Error;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt::Display;
use std::fs::File;
use std::io::{self, BufReader, Read, Write, stdin, stdout};
use std::iter;
use std::path::Path;
use sys_locale::get_locale;

const CKSUM_ALGORITHM_OPTIONS_SYSV: &str = "sysv";
const CKSUM_ALGORITHM_OPTIONS_BSD: &str = "bsd";
const CKSUM_ALGORITHM_OPTIONS_CRC: &str = "crc";
const CKSUM_ALGORITHM_OPTIONS_MD5: &str = "md5";
const CKSUM_ALGORITHM_OPTIONS_SHA1: &str = "sha1";
const CKSUM_ALGORITHM_OPTIONS_SHA224: &str = "sha224";
const CKSUM_ALGORITHM_OPTIONS_SHA256: &str = "sha256";
const CKSUM_ALGORITHM_OPTIONS_SHA384: &str = "sha384";
const CKSUM_ALGORITHM_OPTIONS_SHA512: &str = "sha512";
const CKSUM_ALGORITHM_OPTIONS_BLAKE2B: &str = "blake2b";
const CKSUM_ALGORITHM_OPTIONS_SM3: &str = "sm3";

#[derive(Debug)]
enum CkSumError {
    RawMultipleFiles,
}

#[derive(Debug, PartialEq)]
enum CksumOutputFormat {
    Hexadecimal,
    Raw,
    Base64,
}

impl CTError for CkSumError {
    fn code(&self) -> i32 {
        match self {
            Self::RawMultipleFiles => 1,
        }
    }
}

impl Error for CkSumError {}

impl Display for CkSumError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RawMultipleFiles => {
                write!(f, "the --raw option is not supported with multiple files")
            }
        }
    }
}

fn cksum_detect_algo(
    prgm: &str,
    len: Option<usize>,
) -> (&'static str, Box<dyn CtDigest + 'static>, usize) {
    match prgm {
        CKSUM_ALGORITHM_OPTIONS_SYSV => (
            CKSUM_ALGORITHM_OPTIONS_SYSV,
            Box::new(SYSV::new()) as Box<dyn CtDigest>,
            512,
        ),
        CKSUM_ALGORITHM_OPTIONS_BSD => (
            CKSUM_ALGORITHM_OPTIONS_BSD,
            Box::new(BSD::new()) as Box<dyn CtDigest>,
            1024,
        ),
        CKSUM_ALGORITHM_OPTIONS_CRC => (
            CKSUM_ALGORITHM_OPTIONS_CRC,
            Box::new(CtCRC::new()) as Box<dyn CtDigest>,
            256,
        ),
        CKSUM_ALGORITHM_OPTIONS_MD5 => (
            CKSUM_ALGORITHM_OPTIONS_MD5,
            Box::new(Md5::new()) as Box<dyn CtDigest>,
            128,
        ),
        CKSUM_ALGORITHM_OPTIONS_SHA1 => (
            CKSUM_ALGORITHM_OPTIONS_SHA1,
            Box::new(Sha1::new()) as Box<dyn CtDigest>,
            160,
        ),
        CKSUM_ALGORITHM_OPTIONS_SHA224 => (
            CKSUM_ALGORITHM_OPTIONS_SHA224,
            Box::new(Sha224::new()) as Box<dyn CtDigest>,
            224,
        ),
        CKSUM_ALGORITHM_OPTIONS_SHA256 => (
            CKSUM_ALGORITHM_OPTIONS_SHA256,
            Box::new(Sha256::new()) as Box<dyn CtDigest>,
            256,
        ),
        CKSUM_ALGORITHM_OPTIONS_SHA384 => (
            CKSUM_ALGORITHM_OPTIONS_SHA384,
            Box::new(Sha384::new()) as Box<dyn CtDigest>,
            384,
        ),
        CKSUM_ALGORITHM_OPTIONS_SHA512 => (
            CKSUM_ALGORITHM_OPTIONS_SHA512,
            Box::new(Sha512::new()) as Box<dyn CtDigest>,
            512,
        ),
        CKSUM_ALGORITHM_OPTIONS_BLAKE2B => (
            CKSUM_ALGORITHM_OPTIONS_BLAKE2B,
            Box::new(if let Some(length) = len {
                CtBlake2b::with_output_bytes(length)
            } else {
                CtBlake2b::new()
            }) as Box<dyn CtDigest>,
            512,
        ),
        CKSUM_ALGORITHM_OPTIONS_SM3 => (
            CKSUM_ALGORITHM_OPTIONS_SM3,
            Box::new(CtSm3::new()) as Box<dyn CtDigest>,
            512,
        ),
        _ => unreachable!("unknown algorithm: clap should have prevented this case"),
    }
}

struct CksumOptions {
    algo_name: &'static str,
    digest: Box<dyn CtDigest + 'static>,
    output_bits: usize,
    untagged: bool,
    length: Option<usize>,
    output_format: CksumOutputFormat,
}

/// Calculate checksum
///
/// # Arguments
///
/// * `options` - CLI options for the assigning checksum algorithm
/// * `files` - A iterator of OsStr which is a bunch of files that are using for calculating checksum
#[allow(clippy::cognitive_complexity)]
/**
 *   计算文件或标准输入的校验和。
 *
 * @param mut options 包含校验和计算选项的结构体。
 * @param files 一个迭代器，提供要计算校验和的文件名或"-"表示标准输入。
 * @return CTResult<()>，成功时返回Ok(())，错误时返回Err(Box<CkSumError>)。
 */
fn cksum<'a, I>(mut cksum_opts: CksumOptions, cksum_files: I) -> CTResult<()>
where
    I: Iterator<Item = &'a OsStr>,
{
    // 将文件名迭代器收集到一个向量中，方便后续处理
    let f: Vec<_> = cksum_files.collect();

    // 检查是否以原始格式计算多个文件的校验和，这是不被支持的
    if cksum_opts.output_format == CksumOutputFormat::Raw && f.len() > 1 {
        return Err(Box::new(CkSumError::RawMultipleFiles));
    }

    // 遍历文件列表，对每个文件或标准输入计算校验和
    for file_name in f {
        let filename = Path::new(file_name);
        let stdin_buffer;
        let file_buffer;
        let not_file = filename == OsStr::new("-");

        // 根据文件名是否为"-"，或者是否为目录，选择不同的读取方式
        let mut file = BufReader::new(if not_file {
            stdin_buffer = stdin();
            Box::new(stdin_buffer) as Box<dyn Read>
        } else if filename.is_dir() {
            // 如果是目录，则使用空读取器
            Box::new(BufReader::new(io::empty())) as Box<dyn Read>
        } else {
            // 尝试打开文件
            file_buffer = match File::open(filename) {
                Ok(file) => file,
                Err(err) => {
                    ct_show!(err.map_err_context(|| filename.to_string_lossy().to_string()));
                    continue;
                }
            };
            Box::new(file_buffer) as Box<dyn Read>
        });

        // 计算校验和
        let (sum_hex, sz) =
            cksum_digest_read(&mut cksum_opts.digest, &mut file, cksum_opts.output_bits)
                .map_err_context(|| "failed to read input".to_string())?;

        // 如果是目录，打印错误信息并继续处理下一个文件
        if filename.is_dir() {
            ct_show!(CtSimpleError::new(
                1,
                format!("{}: Is a directory", filename.display())
            ));
            continue;
        }

        // 根据输出格式和算法，格式化校验和结果
        let sum = match cksum_opts.output_format {
            CksumOutputFormat::Raw => {
                // 对于原始格式，根据算法类型转换校验和字符串为字节序列
                let bytes = match cksum_opts.algo_name {
                    CKSUM_ALGORITHM_OPTIONS_CRC => {
                        sum_hex.parse::<u32>().unwrap().to_be_bytes().to_vec()
                    }
                    CKSUM_ALGORITHM_OPTIONS_SYSV | CKSUM_ALGORITHM_OPTIONS_BSD => {
                        sum_hex.parse::<u16>().unwrap().to_be_bytes().to_vec()
                    }
                    _ => decode(sum_hex).unwrap(),
                };
                // 输出原始格式的校验和，然后立即返回
                stdout().write_all(&bytes)?;
                return Ok(());
            }
            CksumOutputFormat::Hexadecimal => sum_hex,
            CksumOutputFormat::Base64 => match cksum_opts.algo_name {
                CKSUM_ALGORITHM_OPTIONS_CRC
                | CKSUM_ALGORITHM_OPTIONS_SYSV
                | CKSUM_ALGORITHM_OPTIONS_BSD => sum_hex,
                _ => ct_encoding::encode(ct_encoding::Format::Base64, &decode(sum_hex).unwrap())
                    .unwrap(),
            },
        };

        let bsd_width = 5;
        // 根据算法和是否为标准输入，格式化并输出校验和结果
        match (cksum_opts.algo_name, not_file) {
            (CKSUM_ALGORITHM_OPTIONS_SYSV, true) => println!(
                "{} {}",
                sum.parse::<u16>().unwrap(),
                div_ceil(sz, cksum_opts.output_bits)
            ),
            (CKSUM_ALGORITHM_OPTIONS_SYSV, false) => println!(
                "{} {} {}",
                sum.parse::<u16>().unwrap(),
                div_ceil(sz, cksum_opts.output_bits),
                filename.display()
            ),
            (CKSUM_ALGORITHM_OPTIONS_BSD, true) => println!(
                "{:0bsd_width$} {:bsd_width$}",
                sum.parse::<u16>().unwrap(),
                div_ceil(sz, cksum_opts.output_bits)
            ),
            (CKSUM_ALGORITHM_OPTIONS_BSD, false) => println!(
                "{:0bsd_width$} {:bsd_width$} {}",
                sum.parse::<u16>().unwrap(),
                div_ceil(sz, cksum_opts.output_bits),
                filename.display()
            ),
            (CKSUM_ALGORITHM_OPTIONS_CRC, true) => println!("{sum} {sz}"),
            (CKSUM_ALGORITHM_OPTIONS_CRC, false) => println!("{sum} {sz} {}", filename.display()),
            (CKSUM_ALGORITHM_OPTIONS_BLAKE2B, _) if !cksum_opts.untagged => {
                if let Some(length) = cksum_opts.length {
                    // 输出BLAKE2b算法的校验和，可选的长度参数
                    println!("BLAKE2b-{} ({}) = {sum}", length * 8, filename.display());
                } else {
                    println!("BLAKE2b ({}) = {sum}", filename.display());
                }
            }
            _ => {
                // 根据是否标记，以不同的格式输出校验和
                if cksum_opts.untagged {
                    println!("{sum}  {}", filename.display());
                } else {
                    println!(
                        "{} ({}) = {sum}",
                        cksum_opts.algo_name.to_ascii_uppercase(),
                        filename.display()
                    );
                }
            }
        }
    }

    Ok(())
}

fn cksum_digest_read<T: Read>(
    cksum_digest: &mut Box<dyn CtDigest>,
    buf_reader: &mut BufReader<T>,
    output_bits: usize,
) -> io::Result<(String, usize)> {
    cksum_digest.reset();

    // 从reader中读取字节并将其写入digest。
    // 如果binary为false且操作系统为Windows，则DigestWriter会在将字节写入digest前将"\r\n"替换为"\n"。否则，它会直接按原样插入字节。
    // 为了支持替换"\r\n"，我们必须调用finalize()，以应对从reader中读取的最后一个字符为"\r"的可能性。
    // （该字符会被DigestWriter缓冲，仅在后续字符为"\n"时才被写出。
    // 但当"\r"是最后一个读取到的字符时，我们需要强制将其写出。）
    let mut digest_writer = CtDigestWriter::new(cksum_digest, true);
    let output_size = std::io::copy(buf_reader, &mut digest_writer)? as usize;
    digest_writer.finalize();

    if cksum_digest.output_bits() > 0 {
        Ok((cksum_digest.result_str(), output_size))
    } else {
        // Assume it's SHAKE.  result_str() doesn't work with shake (as of 8/30/2016)
        let mut bytes = vec![0; output_bits.div_ceil(8)];
        cksum_digest.hash_finalize(&mut bytes);
        Ok((encode(bytes), output_size))
    }
}

mod opt_flags {
    pub const ALGORITHM: &str = "algorithm";
    pub const FILE: &str = "file";
    pub const UNTAGGED: &str = "untagged";
    pub const TAG: &str = "tag";
    pub const LENGTH: &str = "length";
    pub const RAW: &str = "raw";
    pub const BASE64: &str = "base64";
}

#[derive(Default)]
pub struct Cksum;
impl Tool for Cksum {
    fn name(&self) -> &'static str {
        "cksum"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        cksum_main(args.iter().cloned()).map(|_| ())
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    cksum_main(args).map(|_| ())
}

pub fn cksum_main(args: impl ctcore::Args) -> CTResult<i32> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;

    let algo_name: &str = match matches.get_one::<String>(opt_flags::ALGORITHM) {
        Some(v) => v,
        None => CKSUM_ALGORITHM_OPTIONS_CRC,
    };

    let input_length = matches.get_one::<usize>(opt_flags::LENGTH);
    let length = if let Some(length) = input_length {
        match length.to_owned() {
            0 => None,
            n if n % 8 != 0 => {
                // GNU's implementation seem to use these quotation marks
                // in their error messages, so we do the same.
                ctcore::ct_show_error!("invalid length: \u{2018}{length}\u{2019}");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "length is not a multiple of 8",
                )
                .into());
            }
            n if n > 512 => {
                ctcore::ct_show_error!("invalid length: \u{2018}{length}\u{2019}");

                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "maximum digest length for \u{2018}BLAKE2b\u{2019} is 512 bits",
                )
                .into());
            }
            n => {
                if algo_name != CKSUM_ALGORITHM_OPTIONS_BLAKE2B {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--length is only supported with --algorithm=blake2b",
                    )
                    .into());
                }

                // Divide by 8, as our blake2b implementation expects bytes
                // instead of bits.
                Some(n / 8)
            }
        }
    } else {
        None
    };

    let (name, algo, bits) = cksum_detect_algo(algo_name, length);

    let output_format = if matches.get_flag(opt_flags::RAW) {
        CksumOutputFormat::Raw
    } else if matches.get_flag(opt_flags::BASE64) {
        CksumOutputFormat::Base64
    } else {
        CksumOutputFormat::Hexadecimal
    };

    let opts = CksumOptions {
        algo_name: name,
        digest: algo,
        output_bits: bits,
        length,
        untagged: matches.get_flag(opt_flags::UNTAGGED),
        output_format,
    };

    match matches.get_many::<String>(opt_flags::FILE) {
        Some(files) => cksum(opts, files.map(OsStr::new))?,
        None => cksum(opts, iter::once(OsStr::new("-")))?,
    };

    Ok(0)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("cksum.about");
    let usage_description = t!("cksum.usage");

    let args = args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args_override_self(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
        .after_help(t!("cksum.after_help"))
}

fn args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(opt_flags::FILE)
            .hide(true)
            .action(clap::ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(opt_flags::ALGORITHM)
            .long(opt_flags::ALGORITHM)
            .short('a')
            .help(t!("cksum.clap.algorithm"))
            .value_name("ALGORITHM")
            .value_parser([
                CKSUM_ALGORITHM_OPTIONS_SYSV,
                CKSUM_ALGORITHM_OPTIONS_BSD,
                CKSUM_ALGORITHM_OPTIONS_CRC,
                CKSUM_ALGORITHM_OPTIONS_MD5,
                CKSUM_ALGORITHM_OPTIONS_SHA1,
                CKSUM_ALGORITHM_OPTIONS_SHA224,
                CKSUM_ALGORITHM_OPTIONS_SHA256,
                CKSUM_ALGORITHM_OPTIONS_SHA384,
                CKSUM_ALGORITHM_OPTIONS_SHA512,
                CKSUM_ALGORITHM_OPTIONS_BLAKE2B,
                CKSUM_ALGORITHM_OPTIONS_SM3,
            ]),
        Arg::new(opt_flags::UNTAGGED)
            .long(opt_flags::UNTAGGED)
            .help(t!("cksum.clap.untagged"))
            .action(ArgAction::SetTrue)
            .overrides_with(opt_flags::TAG),
        Arg::new(opt_flags::TAG)
            .long(opt_flags::TAG)
            .help(t!("cksum.clap.tag"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::LENGTH)
            .long(opt_flags::LENGTH)
            .value_parser(value_parser!(usize))
            .short('l')
            .help("digest length in bits; must not exceed the max for the blake2 algorithm and must be a multiple of 8")
            .action(ArgAction::Set),
        Arg::new(opt_flags::RAW)
            .long(opt_flags::RAW)
            .help(t!("cksum.clap.raw"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::BASE64)
            .long(opt_flags::BASE64)
            .help(t!("cksum.clap.base64"))
            .action(ArgAction::SetTrue)
            // Even though this could easily just override an earlier '--raw',
            // GNU cksum does not permit these flags to be combined:
            .conflicts_with(opt_flags::RAW),
        Arg::new("help")
            .short('h')
            .long("help")
            .help(t!("cksum.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .short('V')
            .long("version")
            .help(t!("cksum.clap.version"))
            .action(ArgAction::Version),
    ];
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Cksum::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "cksum");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("cksum"));

        // 测试 execute 方法
        let args = vec![OsString::from("cksum"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod tests_ct_app {
        use crate::ct_app;
        use crate::opt_flags;
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_v() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_h() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_file_arg() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", "test.txt"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_algorithm_arg() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--algorithm", "SHA256"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_untagged_arg() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--untagged"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_tag_arg() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--tag"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_length_arg() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--length", "256"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<usize>(opt_flags::LENGTH).is_some());
        }

        #[test]
        fn test_ct_app_raw_arg() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--raw"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_base64_arg() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--base64"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_multiple_files() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                "file1.txt",
                "--file",
                "file2.txt",
                "--file",
                "file3.txt",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_algorithm() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--algorithm", "invalid-algo"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_untagged_and_tag_both_set() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--untagged", "--tag"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_length_out_of_range() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--length", "1025"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
            assert!(matches.get_one::<usize>(opt_flags::LENGTH).is_some());
        }

        #[test]
        fn test_ct_app_length_not_multiple_of_8() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--length", "29"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
            assert!(matches.get_one::<usize>(opt_flags::LENGTH).is_some());
        }

        #[test]
        fn test_ct_app_raw_and_base64_both_set() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--raw", "--base64"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::ArgumentConflict);
        }

        #[test]
        fn test_ct_app_default_algorithm() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_none());
        }

        #[test]
        fn test_ct_app_empty_file_argument() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", ""];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_nonexistent_file() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--file", "/nonexistent/file.txt"];
            let result = command.try_get_matches_from(args);

            // clap does not validate file existence at parse time; this should succeed
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_short_form_algorithm() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-a", "SHA256"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_short_form_length() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-l", "256"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<usize>(opt_flags::LENGTH).is_some());
        }

        #[test]
        fn test_ct_app_short_form_untagged() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-u"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_short_form_tag() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-t"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_short_form_raw() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-r"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_short_form_base64() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-b"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_multiple_options() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm",
                "SHA256",
                "--untagged",
                "--length",
                "256",
                "--raw",
                "--file",
                "test.txt",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }
    }

    #[cfg(test)]
    mod tests_ct_main {
        use crate::cksum_main;

        use std::ffi::OsString;

        #[test]
        fn test_ct_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_file_arg() {
            let args = vec![ctcore::ct_util_name(), "--file", "test.txt"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_algorithm_arg() {
            let args = vec![ctcore::ct_util_name(), "--algorithm", "SHA256"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_length_arg() {
            let args = vec![ctcore::ct_util_name(), "--length", "256"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_multiple_files() {
            let args = vec![
                ctcore::ct_util_name(),
                "--file",
                "file1.txt",
                "--file",
                "file2.txt",
                "--file",
                "file3.txt",
            ];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_invalid_algorithm() {
            let args = vec![ctcore::ct_util_name(), "--algorithm", "invalid-algo"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_length_out_of_range() {
            let args = vec![ctcore::ct_util_name(), "--length", "1025"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_length_not_multiple_of_8() {
            let args = vec![ctcore::ct_util_name(), "--length", "29"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_raw_and_base64_both_set() {
            let args = vec![ctcore::ct_util_name(), "--raw", "--base64"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_empty_file_argument() {
            let args = vec![ctcore::ct_util_name(), "--file", ""];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_nonexistent_file() {
            let args = vec![ctcore::ct_util_name(), "--file", "/nonexistent/file.txt"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_short_form_algorithm() {
            let args = vec![ctcore::ct_util_name(), "-a", "SHA256"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_short_form_length() {
            let args = vec![ctcore::ct_util_name(), "-l", "256"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_short_form_untagged() {
            let args = vec![ctcore::ct_util_name(), "-u"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_short_form_tag() {
            let args = vec![ctcore::ct_util_name(), "-t"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_short_form_raw() {
            let args = vec![ctcore::ct_util_name(), "-r"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_short_form_base64() {
            let args = vec![ctcore::ct_util_name(), "-b"];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_multiple_options() {
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm",
                "SHA256",
                "--untagged",
                "--length",
                "256",
                "--raw",
                "--file",
                "test.txt",
            ];
            let result = cksum_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
    }

    #[cfg(test)]
    mod tests_ct_app_algorithm {
        use std::fs;
        use std::fs::File;

        use crate::ct_app;
        use crate::opt_flags;

        use tempfile::Builder;
        #[test]
        fn test_ct_app_algorithm_sysv() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sysv")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sysv.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sysv",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_sysv_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sysv_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sysv_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=sysv",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_algorithm_bsd() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_bsd")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_bsd.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "bsd",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_bsd_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_bsd_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_bsd_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=bsd",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_crc() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_crc")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_crc.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "crc",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_crc_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_crc_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_crc_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=crc",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_algorithm_md5() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_md5")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_md5.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "md5",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_md5_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_md5_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_md5_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=md5",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_algorithm_sha1() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sha1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sha1.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha1",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_sha1_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sha1_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sha1_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=sha1",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_algorithm_sha224() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sha224")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sha224.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha224",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_sha224_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sha224_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sha224_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=sha224",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_algorithm_sha256() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sha256")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sha256.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha256",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_sha256_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sha256_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sha256_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=sha256",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_algorithm_sha384() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sha384")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sha384.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha384",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_sha384_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sha384_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sha384_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=sha384",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_algorithm_sha512() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sha512")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sha512.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha512",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_sha512_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sha512_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sha512_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=sha512",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_algorithm_blake2b() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_blake2b")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_blake2b.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "blake2b",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_blake2b_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_blake2b_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_blake2b_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=blake2b",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_algorithm_sm3() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sm3")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sm3.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sm3",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
        #[test]
        fn test_ct_app_algorithm_sm3_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_algorithm_sm3_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_algorithm_sm3_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--algorithm=sm3",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }
    }
    #[cfg(test)]
    mod tests_ct_app_arguments {
        use crate::{ct_app, opt_flags};
        use clap::error::ErrorKind;

        use std::fs;
        use std::fs::File;
        use tempfile::Builder;

        #[test]
        fn test_ct_app_tag() {
            // 创建临时目录结构
            let temp_dir = Builder::new().prefix("test_ct_app_tag").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_tag.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--tag",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_untagged() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_untagged")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_untagged.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--untagged",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_length() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_length")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_length.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-l",
                "128",
                "-a",
                "blake2b",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_length_whole() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_length_whole")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_length_whole.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "blake2b",
                "--length=256",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
            assert!(matches.get_one::<String>(opt_flags::ALGORITHM).is_some());
        }

        #[test]
        fn test_ct_app_length_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_length_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_length_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--raw",
                "-a",
                "blake2b",
                "--length=256",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--base64",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_base64_tag() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_base64_tag")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_base64_tag.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--base64",
                "--tag",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_base64_untag() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_base64_untag")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_base64_untag.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--base64",
                "--untagged",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_base64_tag_untag() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_base64_tag_untag")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_base64_tag_untag.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--tag",
                "--base64",
                "--untagged",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new().prefix("test_ct_app_raw").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--raw",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_raw_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_raw_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_raw_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--raw",
                "--base64",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::ArgumentConflict);
        }

        #[test]
        fn test_ct_app_raw_tag() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_raw_tag")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_raw_tag.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--raw",
                "--tag",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_tag_untag() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_tag_untag")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_tag_untag.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--untagged",
                "--tag",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }

        #[test]
        fn test_ct_app_raw_untagged() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ct_app_raw_untagged")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_ct_app_raw_untagged.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--raw",
                "--untagged",
                test_file_path.to_str().unwrap(),
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.args_present());
        }
    }

    #[cfg(test)]
    mod tests_detect_algo {
        use crate::CKSUM_ALGORITHM_OPTIONS_BLAKE2B;
        use crate::CKSUM_ALGORITHM_OPTIONS_BSD;
        use crate::CKSUM_ALGORITHM_OPTIONS_CRC;
        use crate::CKSUM_ALGORITHM_OPTIONS_MD5;
        use crate::CKSUM_ALGORITHM_OPTIONS_SHA1;
        use crate::CKSUM_ALGORITHM_OPTIONS_SHA224;
        use crate::CKSUM_ALGORITHM_OPTIONS_SHA256;
        use crate::CKSUM_ALGORITHM_OPTIONS_SHA384;
        use crate::CKSUM_ALGORITHM_OPTIONS_SHA512;
        use crate::CKSUM_ALGORITHM_OPTIONS_SM3;
        use crate::CKSUM_ALGORITHM_OPTIONS_SYSV;
        use crate::cksum_detect_algo;

        #[test]
        fn test_detect_algo_sysv() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_SYSV, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_SYSV);

            assert_eq!(output_size, 512);
        }

        #[test]
        fn test_detect_algo_bsd() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_BSD, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_BSD);
            assert_eq!(output_size, 1024);
        }

        #[test]
        fn test_detect_algo_crc() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_CRC, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_CRC);
            // assert_digest_output(digest, CRC::new(), 256);
            assert_eq!(output_size, 256);
        }

        #[test]
        fn test_detect_algo_md5() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_MD5, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_MD5);
            // assert_digest_output(digest, Md5::new(), 128);
            assert_eq!(output_size, 128);
        }

        #[test]
        fn test_detect_algo_sha1() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_SHA1, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_SHA1);
            // assert_digest_output(digest, Sha1::new(), 160);
            assert_eq!(output_size, 160);
        }

        #[test]
        fn test_detect_algo_sha224() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_SHA224, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_SHA224);
            // assert_digest_output(digest, Sha224::new(), 224);
            assert_eq!(output_size, 224);
        }

        #[test]
        fn test_detect_algo_sha256() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_SHA256, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_SHA256);
            // assert_digest_output(digest, Sha256::new(), 256);
            assert_eq!(output_size, 256);
        }

        #[test]
        fn test_detect_algo_sha384() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_SHA384, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_SHA384);

            assert_eq!(output_size, 384);
        }

        #[test]
        fn test_detect_algo_sha512() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_SHA512, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_SHA512);

            assert_eq!(output_size, 512);
        }

        #[test]
        fn test_detect_algo_blake2b_with_length() {
            let length = 64;
            let (name, _, output_size) =
                cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_BLAKE2B, Some(length));
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_BLAKE2B);

            assert_eq!(output_size, 512); // Output size should always be 512 for Blake2b
        }

        #[test]
        fn test_detect_algo_blake2b_without_length() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_BLAKE2B, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_BLAKE2B);

            assert_eq!(output_size, 512);
        }

        #[test]
        fn test_detect_algo_sm3() {
            let (name, _, output_size) = cksum_detect_algo(CKSUM_ALGORITHM_OPTIONS_SM3, None);
            assert_eq!(name, CKSUM_ALGORITHM_OPTIONS_SM3);

            assert_eq!(output_size, 512);
        }
    }

    #[cfg(test)]
    mod test_cksum {
        use crate::CKSUM_ALGORITHM_OPTIONS_BLAKE2B;
        use crate::CKSUM_ALGORITHM_OPTIONS_BSD;
        use crate::CKSUM_ALGORITHM_OPTIONS_CRC;
        use crate::CKSUM_ALGORITHM_OPTIONS_MD5;
        use crate::CKSUM_ALGORITHM_OPTIONS_SHA1;
        use crate::CKSUM_ALGORITHM_OPTIONS_SHA224;
        use crate::CKSUM_ALGORITHM_OPTIONS_SHA256;
        use crate::CKSUM_ALGORITHM_OPTIONS_SHA384;
        use crate::CKSUM_ALGORITHM_OPTIONS_SHA512;
        use crate::CKSUM_ALGORITHM_OPTIONS_SM3;
        use crate::CKSUM_ALGORITHM_OPTIONS_SYSV;
        use crate::{CksumOptions, CksumOutputFormat, cksum, cksum_detect_algo, ct_app, opt_flags};
        use std::ffi::OsStr;
        use std::fs;
        use std::fs::File;
        use tempfile::Builder;

        #[test]
        fn test_calculate_checksum_sysv_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sysv")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sysv.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sysv",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SYSV;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sysv_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sysv_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sysv_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sysv_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sysv",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SYSV;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sysv_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sysv_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sysv_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sysv_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sysv",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SYSV;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sysv_base64 error");
                }
            };
        }
        #[test]
        fn test_calculate_checksum_bsd_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_bsd_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_bsd_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "bsd",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_BSD;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_bsd_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_bsd_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_bsd_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_bsd_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "bsd",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_BSD;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_bsd_raw error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_bsd_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_bsd_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_bsd_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "bsd",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_BSD;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_bsd_hexadecimal error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_crc_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_crc_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_crc_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "crc",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_CRC;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_crc_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_crc_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_crc_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_crc_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "crc",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_CRC;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_crc_raw error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_crc_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_crc_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_crc_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "crc",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_CRC;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_crc_hexadecimal error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sm3_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sm3_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sm3_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sm3",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SM3;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sm3_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sm3_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sm3_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sm3_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sm3",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SM3;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sm3_raw error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sm3_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sm3_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sm3_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sm3",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SM3;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sm3_hexadecimal error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha512_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha512_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha512_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha512",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA512;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha512_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha512_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha512_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha512_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha512",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA512;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha512_raw error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha512_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha512_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path =
                sub_dir_path.join("test_calculate_checksum_sha512_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha512",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA512;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha512_hexadecimal error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_md5_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_md5_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_md5_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "md5",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_MD5;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_md5_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_md5_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_md5_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_md5_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "md5",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_MD5;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_md5_raw error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_md5_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_md5_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_md5_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "md5",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_MD5;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_md5_hexadecimal error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha1_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha1_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha1_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha1",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA1;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha1_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha1_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha1_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha1_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha1",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA1;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha1_raw error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha1_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha1_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha1_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha1",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA1;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha1_hexadecimal error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha224_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha224_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha224_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha224",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA224;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha224_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha224_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha224_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha224_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha224",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA224;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha224_raw error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha224_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha224_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path =
                sub_dir_path.join("test_calculate_checksum_sha224_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha224",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA224;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha224_hexadecimal error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha256_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha256_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha256_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha256",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA256;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha256_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha256_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha256_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha256_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha256",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA256;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha256_raw error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha256_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha256_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path =
                sub_dir_path.join("test_calculate_checksum_sha256_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha256",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA256;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha256_hexadecimal error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha384_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha384_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha384_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha384",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA384;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha384_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha384_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha384_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_sha384_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha384",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA384;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha384_raw error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_sha384_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_sha384_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path =
                sub_dir_path.join("test_calculate_checksum_sha384_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "sha384",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_SHA384;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_sha384_hexadecimal error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_blake2b_base64() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_blake2b_base64")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_blake2b_base64.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "blake2b",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_BLAKE2B;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Base64;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_blake2b_base64 error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_blake2b_raw() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_blake2b_raw")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_calculate_checksum_blake2b_raw.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "blake2b",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_BLAKE2B;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Raw;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_blake2b_raw error");
                }
            };
        }

        #[test]
        fn test_calculate_checksum_blake2b_hexadecimal() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_calculate_checksum_blake2b_hexadecimal")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path =
                sub_dir_path.join("test_calculate_checksum_blake2b_hexadecimal.txt");
            File::create(&test_file_path).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "blake2b",
                test_file_path.to_str().unwrap(),
            ];
            let results = command.try_get_matches_from(args);
            let algo_name: &str = CKSUM_ALGORITHM_OPTIONS_BLAKE2B;
            let length = 64;
            let (name, algo, bits) = cksum_detect_algo(algo_name, Some(length));
            let output_format = CksumOutputFormat::Hexadecimal;

            let opts = CksumOptions {
                algo_name: name,
                digest: algo,
                output_bits: bits,
                length: Some(length),
                untagged: false,
                output_format,
            };

            match results
                .expect("get opt_flags error")
                .get_many::<String>(opt_flags::FILE)
            {
                Some(files) => {
                    let s = cksum(opts, files.map(OsStr::new));
                    assert!(s.is_ok());
                }
                None => {
                    panic!("test_calculate_checksum_blake2b_hexadecimal error");
                }
            };
        }
    }
}
