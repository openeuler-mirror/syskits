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
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::{
    ct_encoding,
    ct_error::{CTError, CTResult, CtSimpleError, FromIo, set_ct_exit_code},
    ct_show,
    ct_sum::{
        BSD, CtBlake2b, CtCRC, CtCRC32b, CtDigest, CtDigestWriter, CtSm3, Md5, SYSV, Sha1,
        Sha3_224, Sha3_256, Sha3_384, Sha3_512, Sha224, Sha256, Sha384, Sha512, div_ceil,
    },
};
use hex::decode;
use hex::encode;
use std::error::Error;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt::Display;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write, stdin, stdout};
use std::path::Path;
use sys_locale::get_locale;

const CKSUM_ALGORITHM_OPTIONS_SYSV: &str = "sysv";
const CKSUM_ALGORITHM_OPTIONS_BSD: &str = "bsd";
const CKSUM_ALGORITHM_OPTIONS_CRC: &str = "crc";
const CKSUM_ALGORITHM_OPTIONS_CRC32B: &str = "crc32b";
const CKSUM_ALGORITHM_OPTIONS_MD5: &str = "md5";
const CKSUM_ALGORITHM_OPTIONS_SHA1: &str = "sha1";
const CKSUM_ALGORITHM_OPTIONS_SHA2: &str = "sha2";
const CKSUM_ALGORITHM_OPTIONS_SHA224: &str = "sha224";
const CKSUM_ALGORITHM_OPTIONS_SHA256: &str = "sha256";
const CKSUM_ALGORITHM_OPTIONS_SHA384: &str = "sha384";
const CKSUM_ALGORITHM_OPTIONS_SHA512: &str = "sha512";
const CKSUM_ALGORITHM_OPTIONS_BLAKE2B: &str = "blake2b";
const CKSUM_ALGORITHM_OPTIONS_SM3: &str = "sm3";
const CKSUM_ALGORITHM_OPTIONS_SHA3: &str = "sha3";

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
        CKSUM_ALGORITHM_OPTIONS_CRC32B => (
            CKSUM_ALGORITHM_OPTIONS_CRC32B,
            Box::new(CtCRC32b::new()) as Box<dyn CtDigest>,
            32,
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
            if let Some(length) = len {
                length * 8
            } else {
                512
            },
        ),
        CKSUM_ALGORITHM_OPTIONS_SM3 => (
            CKSUM_ALGORITHM_OPTIONS_SM3,
            Box::new(CtSm3::new()) as Box<dyn CtDigest>,
            256,
        ),
        CKSUM_ALGORITHM_OPTIONS_SHA2 => match len {
            Some(224) => (
                CKSUM_ALGORITHM_OPTIONS_SHA224,
                Box::new(Sha224::new()) as Box<dyn CtDigest>,
                224,
            ),
            Some(256) => (
                CKSUM_ALGORITHM_OPTIONS_SHA256,
                Box::new(Sha256::new()) as Box<dyn CtDigest>,
                256,
            ),
            Some(384) => (
                CKSUM_ALGORITHM_OPTIONS_SHA384,
                Box::new(Sha384::new()) as Box<dyn CtDigest>,
                384,
            ),
            Some(512) | None => (
                CKSUM_ALGORITHM_OPTIONS_SHA512,
                Box::new(Sha512::new()) as Box<dyn CtDigest>,
                512,
            ),
            _ => unreachable!("length should be validated before reaching here"),
        },
        CKSUM_ALGORITHM_OPTIONS_SHA3 => match len {
            Some(224) => (
                "sha3-224",
                Box::new(Sha3_224::new()) as Box<dyn CtDigest>,
                224,
            ),
            Some(256) => (
                "sha3-256",
                Box::new(Sha3_256::new()) as Box<dyn CtDigest>,
                256,
            ),
            Some(384) => (
                "sha3-384",
                Box::new(Sha3_384::new()) as Box<dyn CtDigest>,
                384,
            ),
            Some(512) | None => (
                "sha3-512",
                Box::new(Sha3_512::new()) as Box<dyn CtDigest>,
                512,
            ),
            _ => unreachable!("length should be validated before reaching here"),
        },
        "sha3-224" => (
            "sha3-224",
            Box::new(Sha3_224::new()) as Box<dyn CtDigest>,
            224,
        ),
        "sha3-256" => (
            "sha3-256",
            Box::new(Sha3_256::new()) as Box<dyn CtDigest>,
            256,
        ),
        "sha3-384" => (
            "sha3-384",
            Box::new(Sha3_384::new()) as Box<dyn CtDigest>,
            384,
        ),
        "sha3-512" => (
            "sha3-512",
            Box::new(Sha3_512::new()) as Box<dyn CtDigest>,
            512,
        ),
        _ => unreachable!("unknown algorithm: clap should have prevented this case"),
    }
}

fn detect_algo_from_tag(tag: &str) -> Option<(Box<dyn CtDigest + 'static>, usize, &'static str)> {
    let tag = tag.trim().to_uppercase();

    if let Some(len_str) = tag.strip_prefix("BLAKE2B-") {
        if let Ok(bits) = len_str.parse::<usize>() {
            if bits % 8 == 0 && bits <= 512 {
                return Some((
                    Box::new(CtBlake2b::with_output_bytes(bits / 8)) as Box<dyn CtDigest>,
                    bits,
                    CKSUM_ALGORITHM_OPTIONS_BLAKE2B,
                ));
            }
        }
        return None;
    }

    if let Some(len_str) = tag.strip_prefix("SHA2-") {
        if let Ok(bits) = len_str.parse::<usize>() {
            match bits {
                224 => {
                    return Some((
                        Box::new(Sha224::new()) as Box<dyn CtDigest>,
                        224,
                        CKSUM_ALGORITHM_OPTIONS_SHA224,
                    ));
                }
                256 => {
                    return Some((
                        Box::new(Sha256::new()) as Box<dyn CtDigest>,
                        256,
                        CKSUM_ALGORITHM_OPTIONS_SHA256,
                    ));
                }
                384 => {
                    return Some((
                        Box::new(Sha384::new()) as Box<dyn CtDigest>,
                        384,
                        CKSUM_ALGORITHM_OPTIONS_SHA384,
                    ));
                }
                512 => {
                    return Some((
                        Box::new(Sha512::new()) as Box<dyn CtDigest>,
                        512,
                        CKSUM_ALGORITHM_OPTIONS_SHA512,
                    ));
                }
                _ => return None,
            }
        }
        return None;
    }

    if let Some(len_str) = tag.strip_prefix("SHA3-") {
        if let Ok(bits) = len_str.parse::<usize>() {
            match bits {
                224 => {
                    return Some((
                        Box::new(Sha3_224::new()) as Box<dyn CtDigest>,
                        224,
                        "sha3-224",
                    ));
                }
                256 => {
                    return Some((
                        Box::new(Sha3_256::new()) as Box<dyn CtDigest>,
                        256,
                        "sha3-256",
                    ));
                }
                384 => {
                    return Some((
                        Box::new(Sha3_384::new()) as Box<dyn CtDigest>,
                        384,
                        "sha3-384",
                    ));
                }
                512 => {
                    return Some((
                        Box::new(Sha3_512::new()) as Box<dyn CtDigest>,
                        512,
                        "sha3-512",
                    ));
                }
                _ => return None,
            }
        }
        return None;
    }

    match tag.as_str() {
        "MD5" => Some((
            Box::new(Md5::new()) as Box<dyn CtDigest>,
            128,
            CKSUM_ALGORITHM_OPTIONS_MD5,
        )),
        "SHA1" => Some((
            Box::new(Sha1::new()) as Box<dyn CtDigest>,
            160,
            CKSUM_ALGORITHM_OPTIONS_SHA1,
        )),
        "SHA224" => Some((
            Box::new(Sha224::new()) as Box<dyn CtDigest>,
            224,
            CKSUM_ALGORITHM_OPTIONS_SHA224,
        )),
        "SHA256" => Some((
            Box::new(Sha256::new()) as Box<dyn CtDigest>,
            256,
            CKSUM_ALGORITHM_OPTIONS_SHA256,
        )),
        "SHA384" => Some((
            Box::new(Sha384::new()) as Box<dyn CtDigest>,
            384,
            CKSUM_ALGORITHM_OPTIONS_SHA384,
        )),
        "SHA512" => Some((
            Box::new(Sha512::new()) as Box<dyn CtDigest>,
            512,
            CKSUM_ALGORITHM_OPTIONS_SHA512,
        )),
        "BLAKE2B" => Some((
            Box::new(CtBlake2b::new()) as Box<dyn CtDigest>,
            512,
            CKSUM_ALGORITHM_OPTIONS_BLAKE2B,
        )),
        "SM3" => Some((
            Box::new(CtSm3::new()) as Box<dyn CtDigest>,
            256,
            CKSUM_ALGORITHM_OPTIONS_SM3,
        )),
        "CRC" => Some((
            Box::new(CtCRC::new()) as Box<dyn CtDigest>,
            256,
            CKSUM_ALGORITHM_OPTIONS_CRC,
        )),
        "CRC32B" => Some((
            Box::new(CtCRC32b::new()) as Box<dyn CtDigest>,
            32,
            CKSUM_ALGORITHM_OPTIONS_CRC32B,
        )),
        "SYSV" => Some((
            Box::new(SYSV::new()) as Box<dyn CtDigest>,
            512,
            CKSUM_ALGORITHM_OPTIONS_SYSV,
        )),
        "BSD" => Some((
            Box::new(BSD::new()) as Box<dyn CtDigest>,
            1024,
            CKSUM_ALGORITHM_OPTIONS_BSD,
        )),
        _ => None,
    }
}

struct CksumOptions {
    algo_name: &'static str,
    digest: Box<dyn CtDigest + 'static>,
    output_bits: usize,
    untagged: bool,
    length: Option<usize>,
    output_format: CksumOutputFormat,
    zero: bool,
    binary: bool,
    quiet: bool,
    status: bool,
    warn: bool,
    strict: bool,
    ignore_missing: bool,
}

/// Calculate checksum
///
/// # Arguments
///
/// * `options` - CLI options for the assigning checksum algorithm
/// * `files` - A iterator of OsStr which is a bunch of files that are using for calculating checksum
#[allow(clippy::cognitive_complexity)]
fn cksum<'a, I>(mut cksum_opts: CksumOptions, cksum_files: I) -> CTResult<()>
where
    I: Iterator<Item = &'a OsStr>,
{
    let f: Vec<_> = cksum_files.collect();
    let implicit_stdin = f.is_empty();
    if implicit_stdin {
        let mut stdin_buffer = BufReader::new(stdin());
        let (sum_hex, sz) = cksum_digest_read(
            &mut cksum_opts.digest,
            &mut stdin_buffer,
            cksum_opts.output_bits,
        )
        .map_err_context(|| "failed to read input".to_string())?;

        let line_end = if cksum_opts.zero { "\0" } else { "\n" };

        let sum = match cksum_opts.output_format {
            CksumOutputFormat::Raw => {
                let bytes = match cksum_opts.algo_name {
                    CKSUM_ALGORITHM_OPTIONS_CRC | CKSUM_ALGORITHM_OPTIONS_CRC32B => {
                        sum_hex.parse::<u32>().unwrap().to_be_bytes().to_vec()
                    }
                    CKSUM_ALGORITHM_OPTIONS_SYSV | CKSUM_ALGORITHM_OPTIONS_BSD => {
                        sum_hex.parse::<u16>().unwrap().to_be_bytes().to_vec()
                    }
                    _ => decode(sum_hex).unwrap(),
                };
                stdout().write_all(&bytes)?;
                return Ok(());
            }
            CksumOutputFormat::Hexadecimal => sum_hex,
            CksumOutputFormat::Base64 => match cksum_opts.algo_name {
                CKSUM_ALGORITHM_OPTIONS_CRC
                | CKSUM_ALGORITHM_OPTIONS_CRC32B
                | CKSUM_ALGORITHM_OPTIONS_SYSV
                | CKSUM_ALGORITHM_OPTIONS_BSD => sum_hex,
                _ => ct_encoding::encode(ct_encoding::Format::Base64, &decode(sum_hex).unwrap())
                    .unwrap(),
            },
        };

        let bsd_width = 5;
        match cksum_opts.algo_name {
            CKSUM_ALGORITHM_OPTIONS_SYSV => print!(
                "{} {}{}",
                sum.parse::<u16>().unwrap(),
                div_ceil(sz, cksum_opts.output_bits),
                line_end
            ),
            CKSUM_ALGORITHM_OPTIONS_BSD => print!(
                "{:0bsd_width$} {:bsd_width$}{}",
                sum.parse::<u16>().unwrap(),
                div_ceil(sz, cksum_opts.output_bits),
                line_end
            ),
            CKSUM_ALGORITHM_OPTIONS_CRC | CKSUM_ALGORITHM_OPTIONS_CRC32B => {
                print!("{sum} {sz}{line_end}")
            }
            CKSUM_ALGORITHM_OPTIONS_BLAKE2B if !cksum_opts.untagged => {
                if let Some(length) = cksum_opts.length {
                    print!("BLAKE2b-{} (-) = {sum}{}", length * 8, line_end);
                } else {
                    print!("BLAKE2b (-) = {sum}{line_end}");
                }
            }
            _ => {
                if cksum_opts.untagged {
                    let marker = if cksum_opts.binary { "*" } else { " " };
                    print!("{sum} {marker}-{line_end}");
                } else {
                    print!(
                        "{} (-) = {sum}{}",
                        cksum_opts.algo_name.to_ascii_uppercase(),
                        line_end
                    );
                }
            }
        }

        return Ok(());
    }

    if cksum_opts.output_format == CksumOutputFormat::Raw && f.len() > 1 {
        return Err(Box::new(CkSumError::RawMultipleFiles));
    }

    let line_end = if cksum_opts.zero { "\0" } else { "\n" };

    for file_name in f {
        let filename = Path::new(file_name);
        let stdin_buffer;
        let file_buffer;
        let not_file = filename == OsStr::new("-");

        let mut file = BufReader::new(if not_file {
            stdin_buffer = stdin();
            Box::new(stdin_buffer) as Box<dyn Read>
        } else if filename.is_dir() {
            Box::new(BufReader::new(io::empty())) as Box<dyn Read>
        } else {
            file_buffer = match File::open(filename) {
                Ok(file) => file,
                Err(err) => {
                    ct_show!(err.map_err_context(|| filename.to_string_lossy().to_string()));
                    continue;
                }
            };
            Box::new(file_buffer) as Box<dyn Read>
        });

        let (sum_hex, sz) =
            cksum_digest_read(&mut cksum_opts.digest, &mut file, cksum_opts.output_bits)
                .map_err_context(|| "failed to read input".to_string())?;

        if filename.is_dir() {
            ct_show!(CtSimpleError::new(
                1,
                format!("{}: Is a directory", filename.display())
            ));
            continue;
        }

        let sum = match cksum_opts.output_format {
            CksumOutputFormat::Raw => {
                let bytes = match cksum_opts.algo_name {
                    CKSUM_ALGORITHM_OPTIONS_CRC | CKSUM_ALGORITHM_OPTIONS_CRC32B => {
                        sum_hex.parse::<u32>().unwrap().to_be_bytes().to_vec()
                    }
                    CKSUM_ALGORITHM_OPTIONS_SYSV | CKSUM_ALGORITHM_OPTIONS_BSD => {
                        sum_hex.parse::<u16>().unwrap().to_be_bytes().to_vec()
                    }
                    _ => decode(sum_hex).unwrap(),
                };
                stdout().write_all(&bytes)?;
                return Ok(());
            }
            CksumOutputFormat::Hexadecimal => sum_hex,
            CksumOutputFormat::Base64 => match cksum_opts.algo_name {
                CKSUM_ALGORITHM_OPTIONS_CRC
                | CKSUM_ALGORITHM_OPTIONS_CRC32B
                | CKSUM_ALGORITHM_OPTIONS_SYSV
                | CKSUM_ALGORITHM_OPTIONS_BSD => sum_hex,
                _ => ct_encoding::encode(ct_encoding::Format::Base64, &decode(sum_hex).unwrap())
                    .unwrap(),
            },
        };

        let bsd_width = 5;
        match cksum_opts.algo_name {
            CKSUM_ALGORITHM_OPTIONS_SYSV => print!(
                "{} {} {}{}",
                sum.parse::<u16>().unwrap(),
                div_ceil(sz, cksum_opts.output_bits),
                filename.display(),
                line_end
            ),
            CKSUM_ALGORITHM_OPTIONS_BSD => print!(
                "{:0bsd_width$} {:bsd_width$} {}{}",
                sum.parse::<u16>().unwrap(),
                div_ceil(sz, cksum_opts.output_bits),
                filename.display(),
                line_end
            ),
            CKSUM_ALGORITHM_OPTIONS_CRC | CKSUM_ALGORITHM_OPTIONS_CRC32B => {
                print!("{sum} {sz} {}{}", filename.display(), line_end)
            }
            CKSUM_ALGORITHM_OPTIONS_BLAKE2B if !cksum_opts.untagged => {
                if let Some(length) = cksum_opts.length {
                    print!(
                        "BLAKE2b-{} ({}) = {sum}{}",
                        length * 8,
                        filename.display(),
                        line_end
                    );
                } else {
                    print!("BLAKE2b ({}) = {sum}{}", filename.display(), line_end);
                }
            }
            _ => {
                if cksum_opts.untagged {
                    let marker = if cksum_opts.binary { "*" } else { " " };
                    print!("{sum} {marker}{}{}", filename.display(), line_end);
                } else {
                    print!(
                        "{} ({}) = {sum}{}",
                        cksum_opts.algo_name.to_ascii_uppercase(),
                        filename.display(),
                        line_end
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
    let mut digest_writer = CtDigestWriter::new(cksum_digest, true);
    let output_size = std::io::copy(buf_reader, &mut digest_writer)? as usize;
    digest_writer.finalize();

    if cksum_digest.output_bits() > 0 {
        Ok((cksum_digest.result_str(), output_size))
    } else {
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
    pub const CHECK: &str = "check";
    pub const QUIET: &str = "quiet";
    pub const STATUS: &str = "status";
    pub const IGNORE_MISSING: &str = "ignore-missing";
    pub const STRICT: &str = "strict";
    pub const WARN: &str = "warn";
    pub const ZERO: &str = "zero";
    pub const TEXT: &str = "text";
    pub const BINARY: &str = "binary";
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
        let exit_code = cksum_main(args.iter().cloned())?;
        if exit_code != 0 {
            set_ct_exit_code(exit_code);
        }
        Ok(())
    }
}

pub fn cksum_main(args: impl ctcore::Args) -> CTResult<i32> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let args_vec: Vec<OsString> = args.collect();

    let mut last_tag_idx = 0;
    let mut last_untagged_idx = 0;
    let mut last_binary_idx = 0;
    let mut last_text_idx = 0;
    let mut last_status_idx = 0;
    let mut last_warn_idx = 0;

    let mut untagged = false;
    let mut tag = false;
    let mut binary = false;
    let mut text = false;
    let mut status = false;
    let mut warn = false;

    for (i, arg) in args_vec.iter().enumerate() {
        let arg_str = arg.to_string_lossy();
        if arg_str == "--tag" {
            last_tag_idx = i;
            tag = true;
        } else if arg_str == "--untagged" {
            last_untagged_idx = i;
            untagged = true;
        } else if arg_str == "--binary" || arg_str == "-b" {
            last_binary_idx = i;
            binary = true;
        } else if arg_str == "--text" || arg_str == "-t" {
            last_text_idx = i;
            text = true;
        } else if arg_str == "--status" {
            last_status_idx = i;
            status = true;
        } else if arg_str == "--warn" || arg_str == "-w" {
            last_warn_idx = i;
            warn = true;
        } else if arg_str.starts_with('-') && !arg_str.starts_with("--") {
            if arg_str.contains('b') {
                last_binary_idx = i;
                binary = true;
            }
            if arg_str.contains('t') {
                last_text_idx = i;
                text = true;
            }
            if arg_str.contains('w') {
                last_warn_idx = i;
                warn = true;
            }
        }
    }

    if untagged && tag {
        if last_tag_idx > last_untagged_idx {
            untagged = false;
        } else {
            tag = false;
        }
    }
    if binary && text {
        if last_text_idx > last_binary_idx {
            binary = false;
        } else {
            text = false;
        }
    }
    if binary && last_tag_idx > last_binary_idx {
        binary = false;
    }
    if status && warn && last_status_idx > last_warn_idx {
        warn = false;
    }

    let matches = match ct_app().try_get_matches_from(args_vec) {
        Ok(m) => m,
        Err(e) => {
            let _ = e.print();
            if e.kind() == clap::error::ErrorKind::DisplayHelp
                || e.kind() == clap::error::ErrorKind::DisplayVersion
            {
                return Ok(0);
            }
            return Ok(1);
        }
    };

    let algo_name: &str = match matches.get_one::<String>(opt_flags::ALGORITHM) {
        Some(v) => v,
        None => CKSUM_ALGORITHM_OPTIONS_CRC,
    };

    let input_length_str = matches.get_one::<String>(opt_flags::LENGTH);
    let length = if let Some(len_str) = input_length_str {
        let parsed_n = len_str.parse::<usize>();
        let (is_err, n) = match parsed_n {
            Ok(v) => (false, v),
            Err(_) => (true, 0),
        };

        if !is_err && n == 0 {
            None
        } else {
            match algo_name {
                CKSUM_ALGORITHM_OPTIONS_BLAKE2B => {
                    if is_err || n > 512 {
                        ctcore::ct_show_error!("invalid length: '{}'", len_str);
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "maximum digest length for 'BLAKE2b' is 512 bits",
                        )
                        .into());
                    }
                    if n % 8 != 0 {
                        ctcore::ct_show_error!("invalid length: '{}'", len_str);
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "length is not a multiple of 8",
                        )
                        .into());
                    }
                    Some(n / 8)
                }
                CKSUM_ALGORITHM_OPTIONS_SHA2 | CKSUM_ALGORITHM_OPTIONS_SHA3 => {
                    if is_err || !matches!(n, 224 | 256 | 384 | 512) {
                        ctcore::ct_show_error!("invalid length: '{}'", len_str);
                        let algo_display = if algo_name == CKSUM_ALGORITHM_OPTIONS_SHA2 {
                            "SHA2"
                        } else {
                            "SHA3"
                        };
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!(
                                "digest length for '{}' must be 224, 256, 384, or 512",
                                algo_display
                            ),
                        )
                        .into());
                    }
                    Some(n)
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--length is only supported with --algorithm=blake2b, sha2, or sha3",
                    )
                    .into());
                }
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
        untagged,
        output_format,
        zero: matches.get_flag(opt_flags::ZERO),
        binary,
        quiet: matches.get_flag(opt_flags::QUIET),
        status,
        warn,
        strict: matches.get_flag(opt_flags::STRICT),
        ignore_missing: matches.get_flag(opt_flags::IGNORE_MISSING),
    };

    if matches.get_flag(opt_flags::CHECK) {
        if matches.contains_id(opt_flags::ALGORITHM) {
            if matches!(
                opts.algo_name,
                CKSUM_ALGORITHM_OPTIONS_BSD
                    | CKSUM_ALGORITHM_OPTIONS_SYSV
                    | CKSUM_ALGORITHM_OPTIONS_CRC
                    | CKSUM_ALGORITHM_OPTIONS_CRC32B
            ) {
                ctcore::ct_show_error!(
                    "--check is not supported with --algorithm={}",
                    opts.algo_name
                );
                return Ok(1);
            }
        }

        let files = match matches.get_many::<String>(opt_flags::FILE) {
            Some(v) => v.map(OsStr::new).collect(),
            None => vec![OsStr::new("-")],
        };
        return cksum_check(opts, files);
    }

    match matches.get_many::<String>(opt_flags::FILE) {
        Some(files) => cksum(opts, files.map(OsStr::new))?,
        None => cksum(opts, std::iter::empty())?,
    };

    Ok(0)
}

fn cksum_check(mut opts: CksumOptions, files: Vec<&OsStr>) -> CTResult<i32> {
    let mut global_properly_formatted = 0;
    let mut bad_format = 0;
    let mut bad_checksum = 0;
    let mut missing_files = 0;
    let mut failed_open = 0;
    let mut no_file_verified = false;

    let show_warnings = !opts.status || opts.warn;

    for cksum_file in files {
        let f_name = Path::new(cksum_file);
        let mut n_properly_formatted_this_file = 0;
        let mut n_verified_this_file = 0;
        let mut current_default_algo = opts.algo_name;

        let file_input: Box<dyn BufRead> = if f_name == OsStr::new("-") {
            Box::new(BufReader::new(stdin()))
        } else {
            match File::open(f_name) {
                Ok(f) => Box::new(BufReader::new(f)),
                Err(e) => {
                    let err_msg = e.to_string();
                    let clean_err = err_msg.split(" (os error").next().unwrap_or(&err_msg);
                    ctcore::ct_show_error!("{}: {}", f_name.display(), clean_err);
                    failed_open += 1;
                    continue;
                }
            }
        };

        for (line_num, line_result) in file_input.lines().enumerate() {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => {
                    let err_msg = e.to_string();
                    let clean_err = err_msg.split(" (os error").next().unwrap_or(&err_msg);
                    ctcore::ct_show_error!("{}: {}", f_name.display(), clean_err);
                    continue;
                }
            };

            if line.trim().is_empty() || line.trim().starts_with('#') {
                continue;
            }

            let (digest_str, filename_str, line_algo) = match parse_check_line(&line) {
                Some((d, f, a)) => (d, f, a),
                None => {
                    bad_format += 1;
                    if opts.warn {
                        ctcore::ct_show_error!(
                            "{}: {}: improperly formatted {} checksum line",
                            f_name.display(),
                            line_num + 1,
                            algo_display_name(current_default_algo)
                        );
                    }
                    continue;
                }
            };

            let (current_algo_name, mut current_digest, current_bits) = if let Some(tag) = line_algo
            {
                if let Some((d, b, n)) = detect_algo_from_tag(tag) {
                    current_default_algo = n;
                    (n, d, b)
                } else {
                    bad_format += 1;
                    if opts.warn {
                        let display_tag = {
                            let u = tag.to_uppercase();
                            if u.starts_with("SHA3-") {
                                "SHA3"
                            } else if u.starts_with("SHA2-") {
                                "SHA2"
                            } else if u.starts_with("BLAKE2B-") {
                                "BLAKE2b"
                            } else {
                                tag
                            }
                        };
                        ctcore::ct_show_error!(
                            "{}: {}: improperly formatted {} checksum line",
                            f_name.display(),
                            line_num + 1,
                            display_tag
                        );
                    }
                    continue;
                }
            } else {
                let mut inferred_len = opts.length;

                if inferred_len.is_none() {
                    let is_blake2b = current_default_algo.eq_ignore_ascii_case("blake2b");
                    let is_sha2_family = matches!(
                        current_default_algo,
                        "sha2" | "sha224" | "sha256" | "sha384" | "sha512"
                    );
                    let is_sha3_family = current_default_algo.starts_with("sha3");

                    if is_blake2b || is_sha2_family || is_sha3_family {
                        let mut bits = 0;
                        let is_hex_chars = digest_str.chars().all(|c| c.is_ascii_hexdigit());
                        let is_b64_chars = digest_str.len() % 4 == 0
                            && digest_str.chars().all(|c| {
                                c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='
                            });

                        // 如果包含 [g-z], [G-Z], +, /, = 等非十六进制字符，它必定是 Base64
                        let has_b64_only_chars = digest_str.chars().any(|c| {
                            c == '+'
                                || c == '/'
                                || c == '='
                                || (c >= 'g' && c <= 'z')
                                || (c >= 'G' && c <= 'Z')
                        });

                        if is_b64_chars && has_b64_only_chars {
                            // 通过 base64 的长度和末尾填充的 = 数量，精准逆推哈希字节数
                            let padding =
                                digest_str.chars().rev().take_while(|&c| c == '=').count();
                            let bytes = (digest_str.len() / 4) * 3 - padding;
                            bits = bytes * 8;
                        } else if is_hex_chars {
                            bits = digest_str.len() * 4;
                        } else if is_b64_chars {
                            let padding =
                                digest_str.chars().rev().take_while(|&c| c == '=').count();
                            let bytes = (digest_str.len() / 4) * 3 - padding;
                            bits = bytes * 8;
                        }

                        if bits > 0 {
                            if is_blake2b {
                                inferred_len = Some(bits / 8);
                                current_default_algo = "blake2b";
                            } else if matches!(bits, 224 | 256 | 384 | 512) {
                                inferred_len = Some(bits);
                                current_default_algo = if is_sha2_family { "sha2" } else { "sha3" };
                            }
                        }
                    }
                }

                cksum_detect_algo(current_default_algo, inferred_len)
            };

            if matches!(
                current_algo_name,
                CKSUM_ALGORITHM_OPTIONS_CRC
                    | CKSUM_ALGORITHM_OPTIONS_SYSV
                    | CKSUM_ALGORITHM_OPTIONS_BSD
                    | CKSUM_ALGORITHM_OPTIONS_CRC32B
            ) {
                bad_format += 1;
                if opts.warn {
                    if let Some(tag) = line_algo {
                        let display_tag = {
                            let u = tag.to_uppercase();
                            if u.starts_with("SHA3-") {
                                "SHA3"
                            } else if u.starts_with("SHA2-") {
                                "SHA2"
                            } else if u.starts_with("BLAKE2B-") {
                                "BLAKE2b"
                            } else {
                                tag
                            }
                        };
                        ctcore::ct_show_error!(
                            "{}: {}: improperly formatted {} checksum line",
                            f_name.display(),
                            line_num + 1,
                            display_tag
                        );
                    } else {
                        ctcore::ct_show_error!(
                            "{}: {}: improperly formatted checksum line",
                            f_name.display(),
                            line_num + 1
                        );
                    }
                }
                continue;
            }

            let expected_hex_len = current_bits / 4;
            let expected_b64_len = (((current_bits + 7) / 8) + 2) / 3 * 4;

            let is_hex = digest_str.len() == expected_hex_len
                && digest_str.chars().all(|c| c.is_ascii_hexdigit());
            let is_b64 = digest_str.len() == expected_b64_len
                && digest_str
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=');

            let is_valid_format = is_hex || is_b64;

            if !is_valid_format {
                bad_format += 1;
                if opts.warn {
                    if let Some(tag) = line_algo {
                        let display_tag = {
                            let u = tag.to_uppercase();
                            if u.starts_with("SHA3-") {
                                "SHA3"
                            } else if u.starts_with("SHA2-") {
                                "SHA2"
                            } else if u.starts_with("BLAKE2B-") {
                                "BLAKE2b"
                            } else {
                                tag
                            }
                        };
                        ctcore::ct_show_error!(
                            "{}: {}: improperly formatted {} checksum line",
                            f_name.display(),
                            line_num + 1,
                            display_tag
                        );
                    } else {
                        ctcore::ct_show_error!(
                            "{}: {}: improperly formatted {} checksum line",
                            f_name.display(),
                            line_num + 1,
                            algo_display_name(current_default_algo)
                        );
                    }
                }
                continue;
            }

            n_properly_formatted_this_file += 1;
            global_properly_formatted += 1;

            let target_path = Path::new(filename_str);
            let mut target_file: Box<dyn Read> = match File::open(target_path) {
                Ok(f) => Box::new(BufReader::new(f)),
                Err(e) => {
                    if !opts.ignore_missing {
                        let err_msg = e.to_string();
                        let clean_err = err_msg.split(" (os error").next().unwrap_or(&err_msg);
                        ctcore::ct_show_error!("{}: {}", filename_str, clean_err);
                        if !opts.status {
                            println!(
                                "{}: {}",
                                filename_str,
                                t!("cksum.check.failed_open_or_read")
                            );
                        }
                        missing_files += 1;
                    }
                    continue;
                }
            };

            let (sum_hex, _) = match cksum_digest_read(
                &mut current_digest,
                &mut BufReader::new(&mut target_file),
                current_bits,
            ) {
                Ok(s) => {
                    n_verified_this_file += 1;
                    s
                }
                Err(e) => {
                    if !opts.ignore_missing {
                        let err_msg = e.to_string();
                        let clean_err = err_msg.split(" (os error").next().unwrap_or(&err_msg);
                        ctcore::ct_show_error!("{}: {}", filename_str, clean_err);
                        if !opts.status {
                            println!(
                                "{}: {}",
                                filename_str,
                                t!("cksum.check.failed_open_or_read")
                            );
                        }
                        missing_files += 1;
                    }
                    continue;
                }
            };

            let computed_sum = if is_b64 {
                ct_encoding::encode(ct_encoding::Format::Base64, &decode(&sum_hex).unwrap())
                    .unwrap()
            } else {
                sum_hex
            };

            let checksum_match = if is_b64 {
                computed_sum == digest_str
            } else {
                computed_sum.eq_ignore_ascii_case(digest_str)
            };

            if checksum_match {
                if !opts.quiet && !opts.status {
                    println!("{}: {}", filename_str, t!("cksum.check.ok"));
                }
            } else {
                if !opts.status {
                    println!("{}: {}", filename_str, t!("cksum.check.failed"));
                }
                bad_checksum += 1;
            }
        }

        if n_properly_formatted_this_file == 0 {
            if show_warnings {
                ctcore::ct_show_error!(
                    "{}: no properly formatted checksum lines found",
                    f_name.display()
                );
            }
            no_file_verified = true;
        } else if opts.ignore_missing && n_verified_this_file == 0 {
            if show_warnings {
                ctcore::ct_show_error!(
                    "{}: {}",
                    f_name.display(),
                    t!("cksum.check.no_file_verified")
                );
            }
            no_file_verified = true;
        }
    }

    let mut exit_code = 0;

    if global_properly_formatted > 0 {
        if bad_format > 0 && show_warnings {
            if bad_format == 1 {
                ctcore::ct_show_error!("WARNING: 1 line is improperly formatted");
            } else {
                ctcore::ct_show_error!("WARNING: {} lines are improperly formatted", bad_format);
            }
        }
        if missing_files > 0 && show_warnings {
            if missing_files == 1 {
                ctcore::ct_show_error!("WARNING: 1 listed file could not be read");
            } else {
                ctcore::ct_show_error!("WARNING: {} listed files could not be read", missing_files);
            }
        }
        if bad_checksum > 0 {
            if show_warnings {
                if bad_checksum == 1 {
                    ctcore::ct_show_error!("WARNING: 1 computed checksum did NOT match");
                } else {
                    ctcore::ct_show_error!(
                        "WARNING: {} computed checksums did NOT match",
                        bad_checksum
                    );
                }
            }
            exit_code = 1;
        }

        if bad_format > 0 && opts.strict {
            exit_code = 1;
        }
        if missing_files > 0 {
            exit_code = 1;
        }
    } else {
        if bad_format > 0 || missing_files > 0 || failed_open > 0 || no_file_verified {
            exit_code = 1;
        }
    }

    if missing_files > 0 || failed_open > 0 || no_file_verified {
        exit_code = 1;
    }

    Ok(exit_code)
}

fn parse_check_line(line: &str) -> Option<(&str, &str, Option<&str>)> {
    let mut trimmed = line.trim();

    if let Some(stripped) = trimmed.strip_prefix('\\') {
        trimmed = stripped;
    }

    if let Some(last_paren) = trimmed.rfind(')') {
        let after_paren = &trimmed[last_paren + 1..];
        if let Some(eq_idx) = after_paren.find('=') {
            let digest = after_paren[eq_idx + 1..].trim_start();
            if let Some(first_paren) = trimmed.find('(') {
                if first_paren < last_paren {
                    let algo = trimmed[..first_paren].trim_end();
                    let filename = &trimmed[first_paren + 1..last_paren];
                    return Some((digest, filename, Some(algo)));
                }
            }
        }
    }

    if let Some(first_space) = trimmed.find(' ') {
        let digest = &trimmed[..first_space];
        let rest = trimmed[first_space + 1..].trim_start();
        if let Some(filename) = rest.strip_prefix('*') {
            return Some((digest, filename, None));
        }
        return Some((digest, rest, None));
    }
    None
}

fn algo_display_name(algo: &str) -> &'static str {
    match algo {
        "blake2b" => "BLAKE2b",
        "sm3" => "SM3",
        "md5" => "MD5",
        "sha1" => "SHA1",
        "sha224" => "SHA224",
        "sha256" => "SHA256",
        "sha384" => "SHA384",
        "sha512" => "SHA512",
        "crc" => "CRC",
        "crc32b" => "CRC32b",
        "sysv" => "SYSV",
        "bsd" => "BSD",
        "sha2" => "SHA2",
        "sha3" => "SHA3",
        "sha3-224" => "SHA3-224",
        "sha3-256" => "SHA3-256",
        "sha3-384" => "SHA3-384",
        "sha3-512" => "SHA3-512",
        _ => "UNKNOWN",
    }
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
                CKSUM_ALGORITHM_OPTIONS_CRC32B,
                CKSUM_ALGORITHM_OPTIONS_MD5,
                CKSUM_ALGORITHM_OPTIONS_SHA1,
                CKSUM_ALGORITHM_OPTIONS_SHA224,
                CKSUM_ALGORITHM_OPTIONS_SHA256,
                CKSUM_ALGORITHM_OPTIONS_SHA384,
                CKSUM_ALGORITHM_OPTIONS_SHA512,
                CKSUM_ALGORITHM_OPTIONS_BLAKE2B,
                CKSUM_ALGORITHM_OPTIONS_SM3,
                CKSUM_ALGORITHM_OPTIONS_SHA2,
                CKSUM_ALGORITHM_OPTIONS_SHA3,
            ]),
        Arg::new(opt_flags::UNTAGGED)
            .long(opt_flags::UNTAGGED)
            .help(t!("cksum.clap.untagged"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::TAG)
            .long(opt_flags::TAG)
            .help(t!("cksum.clap.tag"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::LENGTH)
            .long(opt_flags::LENGTH)
            .short('l')
            .help(t!("cksum.clap.length", default = "digest length in bits; must not exceed the max for the blake2 algorithm and must be a multiple of 8"))
            .action(ArgAction::Set),
        Arg::new(opt_flags::RAW)
            .long(opt_flags::RAW)
            .help(t!("cksum.clap.raw"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::BASE64)
            .long(opt_flags::BASE64)
            .help(t!("cksum.clap.base64"))
            .action(ArgAction::SetTrue)
            .conflicts_with(opt_flags::RAW),
        Arg::new(opt_flags::CHECK)
            .short('c')
            .long(opt_flags::CHECK)
            .help(t!("cksum.clap.check", default = "read checksums from the FILEs and check them"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::QUIET)
            .long(opt_flags::QUIET)
            .help(t!("cksum.clap.quiet", default = "don't print OK for each successfully verified file"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::STATUS)
            .long(opt_flags::STATUS)
            .help(t!("cksum.clap.status", default = "don't output anything, status code shows success"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::IGNORE_MISSING)
            .long(opt_flags::IGNORE_MISSING)
            .help(t!("cksum.clap.ignore_missing", default = "don't fail or report status for missing files"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::STRICT)
            .long(opt_flags::STRICT)
            .help(t!("cksum.clap.strict", default = "exit non-zero for improperly formatted checksum lines"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::WARN)
            .short('w')
            .long(opt_flags::WARN)
            .help(t!("cksum.clap.warn", default = "warn about improperly formatted checksum lines"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::ZERO)
            .short('z')
            .long(opt_flags::ZERO)
            .help(t!("cksum.clap.zero", default = "end each output line with NUL, not newline, and disable file name escaping"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::TEXT)
            .short('t')
            .long(opt_flags::TEXT)
            .help(t!("cksum.clap.text", default = "read in text mode"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::BINARY)
            .short('b')
            .long(opt_flags::BINARY)
            .help(t!("cksum.clap.binary", default = "read in binary mode"))
            .action(ArgAction::SetTrue),
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
        let tool = Cksum;

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
            let args = [ctcore::ct_util_name(), "--version"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "-V"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--help"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "-h"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--file", "test.txt"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--algorithm", "SHA256"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--length", "256"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [
                ctcore::ct_util_name(),
                "--file",
                "file1.txt",
                "--file",
                "file2.txt",
                "--file",
                "file3.txt",
            ];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--algorithm", "invalid-algo"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--length", "1025"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--length", "29"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--raw", "--base64"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--file", ""];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "--file", "/nonexistent/file.txt"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "-a", "SHA256"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "-l", "256"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "-u"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "-t"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "-r"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), "-b"];
            let result = cksum_main(args.iter().map(OsString::from));

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
            let args = [
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
            let result = cksum_main(args.iter().map(OsString::from));

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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
                zero: false,
                binary: false,
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
