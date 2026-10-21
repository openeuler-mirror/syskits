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

//! shred 命令的核心实现
//!
//! # 功能概述
//! 该模块实现了类似 GNU shred 的功能，用于安全地删除文件，使其内容无法恢复。
//!
//! # 主要组件
//! - `ShredSettings`: 文件擦除的配置选项
//! - `Pattern`: 擦除模式（单字节或多字节）
//! - `PassType`: 擦除类型（随机或固定模式）
//! - `BytesWriter`: 生成擦除数据的写入器
//! - `ShredFilenameIter`: 生成文件名的迭代器
//!
//! # 核心功能
//! - 多次覆写文件内容
//! - 支持随机数据和固定模式覆写
//! - 文件名混淆
//! - 安全删除
//! - 支持同步写入
//!
//! # 安全性说明
//! 通过多次覆写和文件名混淆，降低数据恢复的可能性。但在某些存储介质上，
//! 可能无法保证数据完全无法恢复。

// spell-checker:ignore (words) wipesync prefill

extern crate rust_i18n;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo, strip_errno};
use ctcore::{ct_show, ct_show_error, ct_show_if_err};
#[cfg(unix)]
use libc::S_IWUSR;
use rand::{Rng, SeedableRng, rngs::StdRng};
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, Write};
#[cfg(unix)]
use std::mem::ManuallyDrop;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use sys_locale::get_locale;

/// shred 命令的选项常量
pub mod shred_options {
    /// 强制写入选项
    pub const SHRED_FORCE: &str = "force";
    /// 输入文件
    pub const SHRED_FILE: &str = "file";
    /// 覆写次数
    pub const SHRED_ITERATIONS: &str = "iterations";
    /// 文件大小
    pub const SHRED_SIZE: &str = "size";
    /// 同步删除选项
    pub const SHRED_WIPESYNC: &str = "u";
    /// 删除方式选项
    pub const SHRED_REMOVE: &str = "remove";
    /// 显示详细信息
    pub const SHRED_VERBOSE: &str = "verbose";
    /// 精确匹配文件大小
    pub const SHRED_EXACT: &str = "exact";
    /// 最后用零填充
    pub const SHRED_ZERO: &str = "zero";
    /// 随机数据源文件
    pub const SHRED_RANDOM_SOURCE: &str = "random-source";

    /// 删除方式的具体选项
    pub mod shred_remove {
        /// 直接删除
        pub const SHRED_UNLINK: &str = "unlink";
        /// 擦除后删除
        pub const SHRED_WIPE: &str = "wipe";
        /// 同步擦除后删除
        pub const SHRED_WIPESYNC: &str = "wipesync";
    }
}

// This block size seems to match GNU (2^16 = 65536)
const SHRED_BLOCK_SIZE: usize = 1 << 16;
const SHRED_NAME_CHARSET: &[u8] =
    b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_.";

const SHRED_PATTERN_LENGTH: usize = 3;

const SHRED_PATTERN_SPEC: &[i32] = &[
    -2, 2, 0x000, 0xFFF, 2, 0x555, 0xAAA, -1, 6, 0x249, 0x492, 0x6DB, 0x924, 0xB6D, 0xDB6, 12,
    0x111, 0x222, 0x333, 0x444, 0x666, 0x777, 0x888, 0x999, 0xBBB, 0xCCC, 0xDDD, 0xEEE, -1, 8,
    0x1000, 0x1249, 0x1492, 0x16DB, 0x1924, 0x1B6D, 0x1DB6, 0x1FFF, 14, 0x1111, 0x1222, 0x1333,
    0x1444, 0x1555, 0x1666, 0x1777, 0x1888, 0x1999, 0x1AAA, 0x1BBB, 0x1CCC, 0x1DDD, 0x1EEE, -1, 0,
];

#[derive(Clone, Copy)]
struct Pattern {
    bytes: [u8; SHRED_PATTERN_LENGTH],
    flip_sector_bit: bool,
}

impl Pattern {
    const fn from_gnu_type(pattern_type: i32) -> Self {
        let bits = (pattern_type as u32) & 0xFFF;
        let repeated = bits | (bits << 12);
        Self {
            bytes: [
                ((repeated >> 4) & 0xFF) as u8,
                ((repeated >> 8) & 0xFF) as u8,
                (repeated & 0xFF) as u8,
            ],
            flip_sector_bit: pattern_type & 0x1000 != 0,
        }
    }
}

enum PassType {
    Pattern(Pattern),
    Random,
    Zero,
}

#[derive(PartialEq, Clone, Copy)]
enum RemoveMethod {
    None,     // Default method. Only obfuscate the file data
    Unlink,   // The same as 'None' + unlink the file
    Wipe,     // The same as 'Unlink' + obfuscate the file name before unlink
    WipeSync, // The same as 'Wipe' sync the file name changes
}

/// Iterates over all possible filenames of a certain length using NAME_CHARSET as an alphabet
struct ShredFilenameIter {
    // Store the indices of the letters of our filename in NAME_CHARSET
    name_charset_indices: Vec<usize>,
    exhausted: bool,
}

impl ShredFilenameIter {
    fn new(name_len: usize) -> Self {
        Self {
            name_charset_indices: vec![0; name_len],
            exhausted: false,
        }
    }
}

impl Iterator for ShredFilenameIter {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        // 如果已经遍历完所有可能的组合，返回 None
        if self.exhausted {
            return None;
        }

        // 根据当前索引生成文件名
        let ret: String = self
            .name_charset_indices
            .iter()
            .map(|i| char::from(SHRED_NAME_CHARSET[*i]))
            .collect();

        // 更新索引，类似于计数器进位
        for index in self.name_charset_indices.iter_mut().rev() {
            if *index == SHRED_NAME_CHARSET.len() - 1 {
                // 当前位已达最大值，重置为0并继续处理下一位
                *index = 0;
                continue;
            } else {
                // 当前位加1
                *index += 1;
                return Some(ret);
            }
        }

        // 所有位都已遍历完，标记为结束
        self.exhausted = true;
        Some(ret)
    }
}

enum RandomReader {
    Entropy(Box<StdRng>),
    File(BufReader<File>),
}

struct RandomSource {
    reader: RandomReader,
    randnum: u64,
    randmax: u64,
}

impl RandomSource {
    fn new(path: Option<&Path>) -> io::Result<Self> {
        let reader = match path {
            Some(path) => RandomReader::File(BufReader::new(File::open(path)?)),
            None => RandomReader::Entropy(Box::new(StdRng::from_entropy())),
        };
        Ok(Self {
            reader,
            randnum: 0,
            randmax: 0,
        })
    }

    fn fill(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        match &mut self.reader {
            RandomReader::Entropy(rng) => {
                rng.fill(bytes);
                Ok(())
            }
            RandomReader::File(reader) => reader.read_exact(bytes),
        }
    }

    fn choose(&mut self, choices: usize) -> io::Result<usize> {
        debug_assert!(choices > 0);
        let genmax = (choices - 1) as u64;
        let mut randnum = self.randnum;
        let mut randmax = self.randmax;

        loop {
            if randmax < genmax {
                let mut count = 0;
                let mut expanded_max = randmax;
                while expanded_max < genmax {
                    expanded_max = expanded_max.wrapping_shl(8).wrapping_add(u8::MAX as u64);
                    count += 1;
                }

                let mut bytes = [0; std::mem::size_of::<u64>()];
                self.fill(&mut bytes[..count])?;
                for byte in bytes.into_iter().take(count) {
                    randnum = randnum.wrapping_shl(8).wrapping_add(byte as u64);
                    randmax = randmax.wrapping_shl(8).wrapping_add(u8::MAX as u64);
                }
            }

            if randmax == genmax {
                self.randnum = 0;
                self.randmax = 0;
                return Ok(randnum as usize);
            }

            let choice_count = genmax + 1;
            let excess_choices = randmax - genmax;
            let unusable_choices = excess_choices % choice_count;
            let last_usable_choice = randmax - unusable_choices;
            let reduced_randnum = randnum % choice_count;
            if randnum <= last_usable_choice {
                self.randnum = randnum / choice_count;
                self.randmax = excess_choices / choice_count;
                return Ok(reduced_randnum as usize);
            }

            randnum = reduced_randnum;
            randmax = unusable_choices - 1;
        }
    }
}

enum BytesWriter<'a> {
    Random {
        source: &'a mut RandomSource,
        buffer: Box<[u8; SHRED_BLOCK_SIZE]>,
    },
    Pattern {
        pattern: Pattern,
        position: usize,
        buffer: Box<[u8; SHRED_BLOCK_SIZE]>,
    },
}

impl<'a> BytesWriter<'a> {
    fn from_pass_type(pass: &PassType, random_source: &'a mut RandomSource) -> Self {
        match pass {
            PassType::Random => Self::Random {
                source: random_source,
                buffer: Box::new([0; SHRED_BLOCK_SIZE]),
            },
            PassType::Pattern(pattern) => Self::Pattern {
                pattern: *pattern,
                position: 0,
                buffer: Box::new([0; SHRED_BLOCK_SIZE]),
            },
            PassType::Zero => Self::Pattern {
                pattern: Pattern::from_gnu_type(0),
                position: 0,
                buffer: Box::new([0; SHRED_BLOCK_SIZE]),
            },
        }
    }

    fn bytes_for_pass(&mut self, size: usize) -> Result<&[u8], io::Error> {
        match self {
            Self::Random { source, buffer } => {
                let bytes = &mut buffer[..size];
                source.fill(bytes)?;
                Ok(bytes)
            }
            Self::Pattern {
                pattern,
                position,
                buffer,
            } => {
                let bytes = &mut buffer[..size];
                for (index, byte) in bytes.iter_mut().enumerate() {
                    let absolute = *position + index;
                    *byte = pattern.bytes[absolute % SHRED_PATTERN_LENGTH];
                    if pattern.flip_sector_bit && absolute % 512 == 0 {
                        *byte ^= 0x80;
                    }
                }
                *position += size;
                Ok(bytes)
            }
        }
    }
}

/// 专门用于 shred 的文件大小解析，支持 0x 和 0 前缀
fn parse_shred_size(size_str: &str) -> Result<u64, ()> {
    let s = size_str.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let had_leading_whitespace = s.len() != size_str.len();
    let (unsigned, had_sign) = if let Some(value) = s.strip_prefix('+') {
        (value, true)
    } else if s.starts_with('-') {
        return Err(());
    } else {
        (s, false)
    };
    if unsigned.is_empty() {
        return Err(());
    }

    let bytes = unsigned.as_bytes();
    let has_hex_prefix = bytes.len() > 2
        && bytes[0] == b'0'
        && matches!(bytes[1], b'x' | b'X')
        && bytes[2].is_ascii_hexdigit();
    let (radix, digit_start) = if has_hex_prefix {
        (16, 2)
    } else if bytes[0] == b'0' {
        (8, 0)
    } else {
        (10, 0)
    };
    let digit_end = bytes[digit_start..]
        .iter()
        .position(|byte| match radix {
            8 => !matches!(byte, b'0'..=b'7'),
            10 => !byte.is_ascii_digit(),
            16 => !byte.is_ascii_hexdigit(),
            _ => unreachable!(),
        })
        .map_or(bytes.len(), |offset| digit_start + offset);

    let (number, suffix) = if digit_end == digit_start {
        if had_sign || had_leading_whitespace {
            return Err(());
        }
        (1, unsigned)
    } else {
        let digits = &unsigned[digit_start..digit_end];
        let number = u64::from_str_radix(digits, radix).map_err(|_| ())?;
        (number, &unsigned[digit_end..])
    };

    let scaled = scale_shred_size(number, suffix)?;
    if scaled > i64::MAX as u64 {
        return Err(());
    }
    Ok(scaled)
}

fn scale_shred_size(mut value: u64, suffix: &str) -> Result<u64, ()> {
    match suffix.as_bytes() {
        b"" | b"c" => return Ok(value),
        b"b" => return value.checked_mul(512).ok_or(()),
        b"B" => return value.checked_mul(1024).ok_or(()),
        _ => {}
    }

    let bytes = suffix.as_bytes();
    let power = match bytes.first() {
        Some(b'k' | b'K') => 1,
        Some(b'M') => 2,
        Some(b'G') => 3,
        Some(b'T') => 4,
        Some(b'P') => 5,
        Some(b'E') => 6,
        Some(b'Z') => 7,
        Some(b'Y') => 8,
        Some(b'R') => 9,
        Some(b'Q') => 10,
        _ => return Err(()),
    };
    let base = match &bytes[1..] {
        b"" | b"iB" => 1024,
        b"B" | b"D" => 1000,
        _ => return Err(()),
    };
    for _ in 0..power {
        value = value.checked_mul(base).ok_or(())?;
    }
    Ok(value)
}

fn parse_shred_iterations(value: &str) -> Result<usize, ()> {
    let value = value.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let value = value.strip_prefix('+').unwrap_or(value);
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }

    let iterations = value.parse::<usize>().map_err(|_| ())?;
    let max_iterations = usize::MAX / std::mem::size_of::<i32>();
    if iterations > max_iterations {
        return Err(());
    }
    Ok(iterations)
}

/// 文件擦除的配置选项
///
/// # 字段说明
/// * `path` - 要擦除的文件路径
/// * `n_passes` - 覆写次数
/// * `remove_method` - 删除方式（无、直接删除、擦除删除、同步擦除删除）
/// * `size` - 指定的文件大小，None 表示使用原始大小
/// * `exact` - 是否精确匹配文件大小
/// * `zero` - 是否在最后用零填充
/// * `verbose` - 是否显示详细信息
/// * `force` - 是否强制写入（修改文件权限）
/// * `random_source` - 随机数据源文件路径
struct ShredSettings<'a> {
    path: &'a Path,
    n_passes: usize,
    remove_method: RemoveMethod,
    size: Option<u64>,
    exact: bool,
    zero: bool,
    verbose: bool,
    force: bool,
    random_source: Option<&'a Path>,
}

impl<'a> ShredSettings<'a> {
    /// 从命令行参数创建配置
    fn new(matches: &'a clap::ArgMatches) -> CTResult<Vec<Self>> {
        // 获取迭代次数
        let n_passes = match matches.get_one::<String>(shred_options::SHRED_ITERATIONS) {
            Some(s) => parse_shred_iterations(s).map_err(|_| {
                CtSimpleError::new(1, format!("invalid number of passes: {}", s.quote()))
            })?,
            None => unreachable!(),
        };

        // 获取删除方法
        let short_remove_index = matches
            .get_flag(shred_options::SHRED_WIPESYNC)
            .then(|| matches.index_of(shred_options::SHRED_WIPESYNC))
            .flatten();
        let long_remove_index = matches.index_of(shred_options::SHRED_REMOVE);
        let remove_method = if long_remove_index > short_remove_index {
            match matches
                .get_one::<String>(shred_options::SHRED_REMOVE)
                .unwrap()
                .as_str()
            {
                shred_options::shred_remove::SHRED_UNLINK => RemoveMethod::Unlink,
                shred_options::shred_remove::SHRED_WIPE => RemoveMethod::Wipe,
                shred_options::shred_remove::SHRED_WIPESYNC => RemoveMethod::WipeSync,
                _ => unreachable!(),
            }
        } else if short_remove_index.is_some() {
            RemoveMethod::WipeSync
        } else {
            RemoveMethod::None
        };

        // 获取文件大小
        let size = matches
            .get_one::<String>(shred_options::SHRED_SIZE)
            .map(|size_str| {
                parse_shred_size(size_str).map_err(|_| {
                    CtSimpleError::new(1, format!("invalid file size: {}", size_str.quote()))
                })
            })
            .transpose()?;

        // 获取其他选项
        let mut exact = matches.get_flag(shred_options::SHRED_EXACT);

        // 如果用户指定了具体的 size，强制进入 exact 模式，不再向上对齐块大小
        if size.is_some() {
            exact = true;
        }

        let zero = matches.get_flag(shred_options::SHRED_ZERO);
        let verbose = matches.get_flag(shred_options::SHRED_VERBOSE);
        let force = matches.get_flag(shred_options::SHRED_FORCE);
        let random_source = matches
            .get_one::<OsString>(shred_options::SHRED_RANDOM_SOURCE)
            .map(Path::new);

        // 获取所有文件路径并创建配置
        let settings = matches
            .get_many::<OsString>(shred_options::SHRED_FILE)
            .unwrap()
            .map(|path| Self {
                path: Path::new(path),
                n_passes,
                remove_method,
                size,
                exact,
                zero,
                verbose,
                force,
                random_source,
            })
            .collect();

        Ok(settings)
    }
}

fn parse_remove_method(value: &str) -> Result<String, String> {
    let methods = [
        shred_options::shred_remove::SHRED_UNLINK,
        shred_options::shred_remove::SHRED_WIPE,
        shred_options::shred_remove::SHRED_WIPESYNC,
    ];

    if methods.contains(&value) {
        return Ok(value.to_string());
    }

    let mut matches = methods
        .into_iter()
        .filter(|method| method.starts_with(value));
    let Some(method) = matches.next() else {
        return Err(format!("invalid remove method: {value}"));
    };
    if matches.next().is_some() {
        return Err(format!("ambiguous remove method: {value}"));
    }

    Ok(method.to_string())
}

pub fn shred_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;

    if !matches.contains_id(shred_options::SHRED_FILE) {
        return Err(CTsageError::new(1, "missing file operand"));
    }

    let settings = ShredSettings::new(&matches)?;
    let mut random_source =
        open_random_source(settings.first().and_then(|setting| setting.random_source))?;

    for setting in settings {
        ct_show_if_err!(shred_exec_with_random_source(&setting, &mut random_source));
    }

    Ok(())
}

fn open_random_source(random_source: Option<&Path>) -> CTResult<RandomSource> {
    RandomSource::new(random_source).map_err_context(|| {
        random_source.map_or_else(
            || "getrandom".to_string(),
            |path| path.maybe_quote().to_string(),
        )
    })
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(shred_options::SHRED_FORCE)
            .long(shred_options::SHRED_FORCE)
            .short('f')
            .help(t!("shred.clap.shred_force"))
            .action(ArgAction::SetTrue),
        Arg::new(shred_options::SHRED_ITERATIONS)
            .long(shred_options::SHRED_ITERATIONS)
            .short('n')
            .help(t!("shred.clap.shred_iterations"))
            .value_name("NUMBER")
            .default_value("3"),
        Arg::new(shred_options::SHRED_SIZE)
            .long(shred_options::SHRED_SIZE)
            .short('s')
            .value_name("N")
            .help(t!("shred.clap.shred_size")),
        Arg::new(shred_options::SHRED_WIPESYNC)
            .short('u')
            .help(t!("shred.clap.shred_wipesync"))
            .action(ArgAction::SetTrue),
        Arg::new(shred_options::SHRED_REMOVE)
            .long(shred_options::SHRED_REMOVE)
            .value_name("HOW")
            .value_parser(parse_remove_method)
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value(shred_options::shred_remove::SHRED_WIPESYNC)
            .help(t!("shred.clap.shred_remove"))
            .action(ArgAction::Set),
        Arg::new(shred_options::SHRED_VERBOSE)
            .long(shred_options::SHRED_VERBOSE)
            .short('v')
            .help(t!("shred.clap.shred_verbose"))
            .action(ArgAction::SetTrue),
        Arg::new(shred_options::SHRED_EXACT)
            .long(shred_options::SHRED_EXACT)
            .short('x')
            .help(
                "do not round file sizes up to the next full block;\n\
                    this is the default for non-regular files",
            )
            .action(ArgAction::SetTrue),
        Arg::new(shred_options::SHRED_ZERO)
            .long(shred_options::SHRED_ZERO)
            .short('z')
            .help(t!("shred.clap.shred_zero"))
            .action(ArgAction::SetTrue),
        Arg::new(shred_options::SHRED_RANDOM_SOURCE)
            .long(shred_options::SHRED_RANDOM_SOURCE)
            .value_name("FILE")
            .help("get random bytes from FILE")
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
        // Positional arguments
        Arg::new(shred_options::SHRED_FILE)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("shred.about"))
        .after_help(t!("shred.after_help"))
        .override_usage(t!("shred.usage"))
        .args_override_self(true)
        .infer_long_args(true)
        .args(&args)
}

fn pass_name(pass_type: &PassType) -> String {
    match pass_type {
        PassType::Random => String::from("random"),
        PassType::Pattern(pattern) => {
            let mut bytes = pattern.bytes;
            if pattern.flip_sector_bit {
                bytes[0] ^= 0x80;
            }
            format!("{:02x}{:02x}{:02x}", bytes[0], bytes[1], bytes[2])
        }
        PassType::Zero => String::from("000000"),
    }
}

/// 执行文件擦除操作
#[cfg(test)]
fn shred_exec(settings: &ShredSettings) -> CTResult<()> {
    let mut random_source = open_random_source(settings.random_source)?;
    shred_exec_with_random_source(settings, &mut random_source)
}

fn shred_exec_with_random_source(
    settings: &ShredSettings,
    random_source: &mut RandomSource,
) -> CTResult<()> {
    if settings.path == Path::new("-") {
        return shred_exec_stdout(settings, random_source);
    }

    let path = settings.path;
    let mut file = shred_open_for_writing(path, settings.force)?;
    let metadata = file
        .metadata()
        .map_err_context(|| format!("{}: fstat failed", path.maybe_quote()))?;
    validate_file_type(&file, path, &metadata)?;
    let size = shred_target_size(settings, &metadata);
    let inode_size = shred_inode_pass_size(settings, &metadata, size);

    let pass_result = shred_run_passes(
        &mut file,
        settings.path,
        settings,
        inode_size,
        size,
        true,
        random_source,
    );

    let truncate_result = if pass_result.is_ok() && settings.remove_method != RemoveMethod::None {
        match file.set_len(0) {
            Err(err) if metadata.is_file() => Err(err),
            _ => Ok(()),
        }
    } else {
        Ok(())
    };
    let close_result = shred_close_file(file);

    pass_result?;
    truncate_result
        .map_err_context(|| format!("{}: error truncating", settings.path.maybe_quote()))?;
    close_result.map_err_context(|| format!("{}: failed to close", settings.path.maybe_quote()))?;

    // 如果需要，删除文件
    if settings.remove_method != RemoveMethod::None {
        shred_do_remove(
            path,
            settings.path,
            settings.verbose,
            settings.remove_method,
        )?;
    }
    Ok(())
}

#[cfg(unix)]
fn shred_close_file(file: File) -> io::Result<()> {
    let fd = file.into_raw_fd();
    if unsafe { libc::close(fd) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn shred_close_file(_file: File) -> io::Result<()> {
    Ok(())
}

fn shred_target_size(settings: &ShredSettings, metadata: &fs::Metadata) -> u64 {
    let size = settings.size.unwrap_or(metadata.len());
    if settings.size.is_none() && !settings.exact {
        round_up_to_block_size(size, shred_file_system_block_size(metadata))
    } else {
        size
    }
}

fn shred_inode_pass_size(
    settings: &ShredSettings,
    metadata: &fs::Metadata,
    target_size: u64,
) -> u64 {
    if !metadata.is_file() {
        return 0;
    }

    let original_size = metadata.len();
    let block_size = shred_file_system_block_size(metadata);
    if settings.size.is_none() {
        if !settings.exact && original_size < block_size {
            original_size
        } else {
            0
        }
    } else if original_size < block_size.min(target_size) {
        original_size
    } else {
        0
    }
}

#[cfg(unix)]
fn shred_file_system_block_size(metadata: &fs::Metadata) -> u64 {
    metadata.blksize().max(1)
}

#[cfg(not(unix))]
fn shred_file_system_block_size(_metadata: &fs::Metadata) -> u64 {
    SHRED_BLOCK_SIZE as u64
}

fn round_up_to_block_size(size: u64, block_size: u64) -> u64 {
    if size == 0 {
        return 0;
    }

    let remainder = size % block_size;
    if remainder == 0 {
        size
    } else {
        size.saturating_add(block_size - remainder)
    }
}

fn shred_open_for_writing(path: &Path, force: bool) -> CTResult<File> {
    match OpenOptions::new().write(true).truncate(false).open(path) {
        Ok(file) => Ok(file),
        Err(err) if force && err.kind() == io::ErrorKind::PermissionDenied => {
            let metadata = fs::metadata(path).map_err_context(String::new)?;
            shred_force_writable(path, &metadata)?;
            OpenOptions::new()
                .write(true)
                .truncate(false)
                .open(path)
                .map_err_context(|| format!("{}: failed to open for writing", path.maybe_quote()))
        }
        Err(err) => Err(err)
            .map_err_context(|| format!("{}: failed to open for writing", path.maybe_quote())),
    }
}

#[cfg(unix)]
fn validate_file_type(file: &File, path: &Path, metadata: &fs::Metadata) -> CTResult<()> {
    let file_type = metadata.file_type();
    let is_tty = file_type.is_char_device() && unsafe { libc::isatty(file.as_raw_fd()) == 1 };
    if is_tty || file_type.is_fifo() || file_type.is_socket() {
        return Err(CtSimpleError::new(
            1,
            format!("{}: invalid file type", path.maybe_quote()),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_file_type(_file: &File, _path: &Path, _metadata: &fs::Metadata) -> CTResult<()> {
    Ok(())
}

fn shred_force_writable(path: &Path, metadata: &fs::Metadata) -> CTResult<()> {
    let mut perms = metadata.permissions();
    #[cfg(unix)]
    {
        perms.set_mode(S_IWUSR);
    }
    #[cfg(not(unix))]
    perms.set_readonly(false);
    fs::set_permissions(path, perms).map_err_context(String::new)
}

#[cfg(unix)]
fn shred_exec_stdout(settings: &ShredSettings, random_source: &mut RandomSource) -> CTResult<()> {
    let stat = stdout_stat()?;
    validate_stdout_fd(stat.st_mode)?;

    let current_size = u64::try_from(stat.st_size).unwrap_or(0);
    let stdout_is_regular = stdout_file_type(stat.st_mode) == libc::S_IFREG;
    let mut size = settings.size.unwrap_or(current_size);
    if settings.size.is_none() && !settings.exact && stdout_is_regular {
        size = round_up_to_block_size(size, u64::try_from(stat.st_blksize).unwrap_or(1).max(1));
    }

    // Borrow stdout without reopening it or closing the inherited descriptor.
    let mut file = ManuallyDrop::new(unsafe { File::from_raw_fd(libc::STDOUT_FILENO) });

    shred_run_passes(
        &mut file,
        Path::new("-"),
        settings,
        0,
        size,
        stdout_is_regular,
        random_source,
    )?;

    if settings.remove_method != RemoveMethod::None {
        file.set_len(0)
            .map_err_context(|| "-: error truncating".to_string())?;
    }

    Ok(())
}

#[cfg(not(unix))]
fn shred_exec_stdout(_settings: &ShredSettings, _random_source: &mut RandomSource) -> CTResult<()> {
    Err(CtSimpleError::new(1, "-: invalid file type"))
}

fn shred_pass_sequence(
    settings: &ShredSettings,
    random_source: &mut RandomSource,
) -> io::Result<Vec<PassType>> {
    let mut pass_sequence = generate_pass_types(settings.n_passes, random_source)?
        .into_iter()
        .map(|pattern_type| {
            if pattern_type < 0 {
                PassType::Random
            } else {
                PassType::Pattern(Pattern::from_gnu_type(pattern_type))
            }
        })
        .collect::<Vec<_>>();

    if settings.zero {
        pass_sequence.push(PassType::Zero);
    }

    Ok(pass_sequence)
}

fn generate_pass_types(num: usize, random_source: &mut RandomSource) -> io::Result<Vec<i32>> {
    let mut passes = vec![0; num];
    if num == 0 {
        return Ok(passes);
    }

    let mut spec_index = 0;
    let mut random_passes = 0;
    let mut destination = 0;
    let mut remaining = num;

    loop {
        let mut group_size = SHRED_PATTERN_SPEC[spec_index];
        spec_index += 1;
        if group_size == 0 {
            spec_index = 0;
        } else if group_size < 0 {
            group_size = -group_size;
            if group_size as usize >= remaining {
                random_passes += remaining;
                break;
            }
            random_passes += group_size as usize;
            remaining -= group_size as usize;
        } else if group_size as usize <= remaining {
            let group_size = group_size as usize;
            passes[destination..destination + group_size]
                .copy_from_slice(&SHRED_PATTERN_SPEC[spec_index..spec_index + group_size]);
            spec_index += group_size;
            destination += group_size;
            remaining -= group_size;
        } else if remaining < 2 || 3 * remaining < group_size as usize {
            random_passes += remaining;
            break;
        } else {
            while remaining > 0 {
                if remaining == group_size as usize
                    || random_source.choose(group_size as usize)? < remaining
                {
                    passes[destination] = SHRED_PATTERN_SPEC[spec_index];
                    destination += 1;
                    remaining -= 1;
                }
                spec_index += 1;
                group_size -= 1;
            }
            break;
        }
    }

    let mut top = num - random_passes;
    let random_step = random_passes - 1;
    let mut accumulator = random_step;
    for index in 0..num {
        if accumulator <= random_step {
            accumulator += num - 1;
            passes[top] = passes[index];
            top += 1;
            passes[index] = -1;
        } else {
            let swap = index + random_source.choose(top - index)?;
            passes.swap(index, swap);
        }
        accumulator -= random_step;
    }

    Ok(passes)
}

fn shred_run_passes(
    file: &mut File,
    display_name: &Path,
    settings: &ShredSettings,
    inode_size: u64,
    size: u64,
    sync_data: bool,
    random_source: &mut RandomSource,
) -> CTResult<()> {
    let pass_sequence = shred_pass_sequence(settings, random_source).map_err_context(|| {
        settings.random_source.map_or_else(
            || "getrandom".to_string(),
            |path| path.maybe_quote().to_string(),
        )
    })?;
    if inode_size == 0 && size == 0 {
        return Ok(());
    }
    let total_passes = pass_sequence.len();
    let mut pass_failed = false;

    for (pass_size, verbose) in [(inode_size, false), (size, settings.verbose)] {
        if pass_size == 0 {
            continue;
        }
        for (i, pass_type) in pass_sequence.iter().enumerate() {
            if verbose {
                let pass_name = pass_name(pass_type);
                ct_show_error!(
                    "{}: pass {}/{} ({})...",
                    display_name.maybe_quote(),
                    i + 1,
                    total_passes,
                    pass_name
                );
            }
            let pass_result = if sync_data {
                shred_do_pass(file, pass_type, settings.exact, pass_size, random_source)
            } else {
                shred_do_pass_impl(
                    file,
                    pass_type,
                    settings.exact,
                    pass_size,
                    random_source,
                    false,
                )
            };
            if let Err(err) = pass_result.map_err_context(|| display_name.maybe_quote().to_string())
            {
                ct_show!(err);
                pass_failed = true;
            }
        }
    }

    if pass_failed {
        Err(CtSimpleError::new(1, String::new()))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn stdout_stat() -> CTResult<libc::stat> {
    unsafe {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if libc::fstat(libc::STDOUT_FILENO, stat.as_mut_ptr()) != 0 {
            let err = io::Error::last_os_error();
            return Err(CtSimpleError::new(
                1,
                format!("-: fstat failed: {}", strip_errno(&err)),
            ));
        }
        Ok(stat.assume_init())
    }
}

#[cfg(unix)]
fn validate_stdout_fd(mode: libc::mode_t) -> CTResult<()> {
    let fd_flags = unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_GETFL) };
    if fd_flags < 0 {
        let err = io::Error::last_os_error();
        return Err(CtSimpleError::new(
            1,
            format!("-: fcntl failed: {}", strip_errno(&err)),
        ));
    }

    if fd_flags & libc::O_APPEND != 0 {
        return Err(CtSimpleError::new(
            1,
            "-: cannot shred append-only file descriptor",
        ));
    }

    let file_type = stdout_file_type(mode);
    let stdout_is_tty =
        file_type == libc::S_IFCHR && unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 };
    if stdout_is_tty || file_type == libc::S_IFIFO || file_type == libc::S_IFSOCK {
        return Err(CtSimpleError::new(1, "-: invalid file type"));
    }

    Ok(())
}

#[cfg(unix)]
fn stdout_file_type(mode: libc::mode_t) -> libc::mode_t {
    mode & libc::S_IFMT
}

/// 执行单次文件覆写
///
/// # 参数
/// * `file` - 要覆写的文件
/// * `pass_type` - 覆写模式（随机或固定模式）
/// * `exact` - 是否精确匹配文件大小
/// * `file_size` - 要覆写的字节数
/// * `random_source` - 随机数据源文件路径
///
/// # 返回值
/// 成功返回 Ok(())，失败返回 IO 错误
fn shred_do_pass(
    file: &mut File,
    pass_type: &PassType,
    _exact: bool,
    file_size: u64,
    random_source: &mut RandomSource,
) -> Result<(), io::Error> {
    shred_do_pass_impl(file, pass_type, _exact, file_size, random_source, true)
}

fn shred_do_pass_impl(
    file: &mut File,
    pass_type: &PassType,
    _exact: bool,
    file_size: u64,
    random_source: &mut RandomSource,
    sync_data: bool,
) -> Result<(), io::Error> {
    // 重置文件指针到开始位置
    file.rewind()?;

    let mut writer = BytesWriter::from_pass_type(pass_type, random_source);
    let mut offset = 0u64;

    // 按块写入数据
    for _ in 0..(file_size / SHRED_BLOCK_SIZE as u64) {
        let block = writer.bytes_for_pass(SHRED_BLOCK_SIZE)?;
        shred_write_all(file, block, offset)?;
        offset += block.len() as u64;
    }

    // 处理剩余字节
    let bytes_left = (file_size % SHRED_BLOCK_SIZE as u64) as usize;
    if bytes_left > 0 {
        let block = writer.bytes_for_pass(bytes_left)?;
        shred_write_all(file, block, offset)?;
    }

    // 确保数据写入磁盘
    if sync_data {
        file.sync_data()?;
    }

    Ok(())
}

fn shred_write_all(file: &mut File, mut bytes: &[u8], mut offset: u64) -> io::Result<()> {
    while !bytes.is_empty() {
        match file.write(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    format!("error writing at offset {offset}"),
                ));
            }
            Ok(written) => {
                bytes = &bytes[written..];
                offset += written as u64;
            }
            Err(err) => {
                return Err(io::Error::new(
                    err.kind(),
                    format!("error writing at offset {offset}: {}", strip_errno(&err)),
                ));
            }
        }
    }
    Ok(())
}

/// 通过重命名擦除文件名
///
/// # 参数
/// * `orig_path` - 原始文件路径
/// * `verbose` - 是否显示详细信息
/// * `remove_method` - 删除方法
///
/// # 返回值
/// 成功返回最终文件路径，失败返回 None
fn shred_wipe_name(
    orig_path: &Path,
    verbose: bool,
    remove_method: RemoveMethod,
) -> Option<PathBuf> {
    let file_name_len = orig_path.file_name()?.len();
    let mut last_path = PathBuf::from(orig_path);
    let parent = orig_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_dir = (remove_method == RemoveMethod::WipeSync)
        .then(|| File::open(parent).ok())
        .flatten();

    // 从长到短尝试不同长度的文件名
    'outer: for length in (1..=file_name_len).rev() {
        // 尝试该长度的所有可能文件名
        for name in ShredFilenameIter::new(length) {
            let new_path = orig_path.with_file_name(name);

            // 尝试重命名
            match shred_rename_noreplace(&last_path, &new_path) {
                Ok(()) => {
                    if verbose {
                        ct_show_error!(
                            "{}: renamed to {}",
                            last_path.maybe_quote(),
                            new_path.display()
                        );
                    }

                    if let Some(parent_dir) = parent_dir.as_ref() {
                        if let Err(err) = shred_sync_directory(parent_dir) {
                            ct_show_error!(
                                "{}: fdatasync failed: {}",
                                parent.display(),
                                strip_errno(&err)
                            );
                            return None;
                        }
                    }

                    last_path = new_path;
                    continue 'outer;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    ct_show_error!(
                        "{}: Couldn't rename to {}: {}",
                        last_path.maybe_quote(),
                        new_path.quote(),
                        strip_errno(&e)
                    );
                    return None;
                }
            }
        }
    }

    Some(last_path)
}

#[cfg(target_os = "linux")]
fn shred_rename_noreplace(old_path: &Path, new_path: &Path) -> io::Result<()> {
    let old_path = CString::new(old_path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let new_path = CString::new(new_path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            old_path.as_ptr(),
            libc::AT_FDCWD,
            new_path.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
fn shred_rename_noreplace(old_path: &Path, new_path: &Path) -> io::Result<()> {
    fs::rename(old_path, new_path)
}

fn shred_sync_directory(directory: &File) -> io::Result<()> {
    match directory.sync_data() {
        Ok(()) => Ok(()),
        Err(err)
            if matches!(
                err.raw_os_error(),
                Some(libc::EINVAL | libc::EBADF | libc::EISDIR)
            ) =>
        {
            directory.sync_all()
        }
        Err(err) => Err(err),
    }
}

/// 删除文件
///
/// # 参数
/// * `path` - 文件路径
/// * `orig_filename` - 原始文件名
/// * `verbose` - 是否显示详细信息
/// * `remove_method` - 删除方法
///
/// # 返回值
/// 成功返回 Ok(())，失败返回 IO 错误
fn shred_do_remove(
    path: &Path,
    orig_filename: &Path,
    verbose: bool,
    remove_method: RemoveMethod,
) -> CTResult<()> {
    if verbose {
        ct_show_error!("{}: removing", orig_filename.maybe_quote());
    }

    // 根据删除方法选择最终路径
    let remove_path = if remove_method == RemoveMethod::Unlink {
        Some(path.to_path_buf())
    } else {
        shred_wipe_name(path, verbose, remove_method)
    };

    // 删除文件
    if let Some(rp) = remove_path {
        if let Err(e) = fs::remove_file(&rp) {
            return Err(io::Error::new(
                e.kind(),
                format!(
                    "{}: failed to remove file: {}",
                    rp.maybe_quote(),
                    strip_errno(&e)
                ),
            )
            .into());
        }
        if remove_method == RemoveMethod::WipeSync {
            let parent = path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            if let Ok(parent_dir) = File::open(parent) {
                shred_sync_directory(&parent_dir)
                    .map_err_context(|| format!("{}: fdatasync failed", parent.display()))?;
            }
        }
    } else {
        return Err(CtSimpleError::new(1, String::new()));
    }

    if verbose {
        ct_show_error!("{}: removed", orig_filename.maybe_quote());
    }

    Ok(())
}

#[derive(Default)]
pub struct Shred;
impl Tool for Shred {
    fn name(&self) -> &'static str {
        "shred"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        shred_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    use tempfile::Builder;
    use tempfile::tempdir;

    #[test]
    fn test_tool_implementation() {
        let tool = Shred;

        // 测试 name 方法
        assert_eq!(tool.name(), "shred");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("shred"));

        // 测试 execute 方法
        let args = vec![OsString::from("shred"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err()); // shred命令需要必要参数，所以测试不带参数的情况应该返回错误
    }

    #[test]
    fn test_app_random_source_arg() {
        let app = ct_app();

        // 测试 --random-source 参数
        let args = vec!["shred", "--random-source", "/dev/urandom", "testfile.txt"];
        let matches = app.try_get_matches_from(args);
        assert!(matches.is_ok(), "应该能解析 --random-source 参数");

        let matches = matches.unwrap();
        assert!(
            matches.contains_id(shred_options::SHRED_RANDOM_SOURCE),
            "应该包含 random-source 参数"
        );
        assert_eq!(
            matches
                .get_one::<OsString>(shred_options::SHRED_RANDOM_SOURCE)
                .unwrap(),
            &OsString::from("/dev/urandom"),
            "随机源路径应该正确解析"
        );
    }

    #[test]
    fn test_settings_with_random_source() -> CTResult<()> {
        let app = ct_app();
        let args = vec![
            "shred",
            "--random-source",
            "/dev/urandom",
            "--iterations",
            "2",
            "testfile.txt",
        ];
        let matches = app.try_get_matches_from(args).unwrap();

        let settings = ShredSettings::new(&matches)?;
        assert_eq!(settings.len(), 1, "应该有一个文件设置");

        let setting = &settings[0];
        assert_eq!(
            setting.random_source,
            Some(Path::new("/dev/urandom")),
            "随机源应该正确设置"
        );
        assert_eq!(setting.n_passes, 2, "迭代次数应该正确设置");
        assert_eq!(
            setting.path,
            Path::new("testfile.txt"),
            "文件路径应该正确设置"
        );

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn test_accepts_non_tty_character_device() -> CTResult<()> {
        shred_main(
            [
                OsString::from("shred"),
                OsString::from("-n0"),
                OsString::from("/dev/null"),
            ]
            .into_iter(),
        )
    }

    #[test]
    fn test_continues_after_file_error() -> CTResult<()> {
        let dir = tempdir().unwrap();
        let second = dir.path().join("second");
        fs::write(&second, b"data").unwrap();

        shred_main(
            [
                OsString::from("shred"),
                OsString::from("-n0"),
                OsString::from("--remove=unlink"),
                dir.path().join("missing").into_os_string(),
                second.clone().into_os_string(),
            ]
            .into_iter(),
        )?;

        assert!(!second.exists());
        Ok(())
    }

    #[test]
    fn test_validates_random_source_before_files() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, b"data").unwrap();

        let result = shred_main(
            [
                OsString::from("shred"),
                OsString::from("-n0"),
                OsString::from("--random-source"),
                dir.path().join("missing-source").into_os_string(),
                target.clone().into_os_string(),
            ]
            .into_iter(),
        );

        assert!(result.is_err());
        assert_eq!(fs::read(target).unwrap(), b"data");
    }

    #[test]
    fn test_repeated_options_use_last_value() {
        let matches = ct_app()
            .try_get_matches_from([
                "shred",
                "-n",
                "1",
                "-n",
                "2",
                "-s",
                "1",
                "-s",
                "2",
                "-v",
                "-v",
                "--random-source=/dev/zero",
                "--random-source=/dev/zero",
                "testfile",
            ])
            .unwrap();

        assert_eq!(
            matches.get_one::<String>(shred_options::SHRED_ITERATIONS),
            Some(&"2".to_string())
        );
        assert_eq!(
            matches.get_one::<String>(shred_options::SHRED_SIZE),
            Some(&"2".to_string())
        );
        assert!(matches.get_flag(shred_options::SHRED_VERBOSE));
        assert_eq!(
            matches.get_one::<OsString>(shred_options::SHRED_RANDOM_SOURCE),
            Some(&OsString::from("/dev/zero"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_non_utf8_file_and_random_source_paths() -> CTResult<()> {
        use std::os::unix::ffi::OsStringExt;

        let dir = tempdir().unwrap();
        let target = dir.path().join(OsString::from_vec(b"target-\xff".to_vec()));
        let random_source = dir.path().join(OsString::from_vec(b"source-\xfe".to_vec()));
        fs::write(&target, b"data").unwrap();
        fs::write(&random_source, b"ZZZZ").unwrap();

        shred_main(
            vec![
                OsString::from("shred"),
                OsString::from("-x"),
                OsString::from("-n1"),
                OsString::from("--random-source"),
                random_source.into_os_string(),
                target.clone().into_os_string(),
            ]
            .into_iter(),
        )?;
        assert_eq!(fs::read(&target).unwrap(), b"ZZZZ");

        let removable = dir.path().join(OsString::from_vec(b"remove-\xfd".to_vec()));
        fs::write(&removable, b"data").unwrap();
        shred_main(
            vec![
                OsString::from("shred"),
                OsString::from("-n0"),
                OsString::from("--remove=wipe"),
                removable.clone().into_os_string(),
            ]
            .into_iter(),
        )?;
        assert!(!removable.exists());

        Ok(())
    }

    #[test]
    fn test_remove_options_use_last_occurrence() -> CTResult<()> {
        let matches = ct_app()
            .try_get_matches_from(["shred", "-u", "--remove=unlink", "testfile"])
            .unwrap();
        let settings = ShredSettings::new(&matches)?;
        assert!(settings[0].remove_method == RemoveMethod::Unlink);

        let matches = ct_app()
            .try_get_matches_from(["shred", "--remove=unlink", "-u", "testfile"])
            .unwrap();
        let settings = ShredSettings::new(&matches)?;
        assert!(settings[0].remove_method == RemoveMethod::WipeSync);

        Ok(())
    }

    #[test]
    fn test_remove_method_unique_abbreviations() {
        assert_eq!(parse_remove_method("u").unwrap(), "unlink");
        assert_eq!(parse_remove_method("unl").unwrap(), "unlink");
        assert_eq!(parse_remove_method("wipe").unwrap(), "wipe");
        assert_eq!(parse_remove_method("wipes").unwrap(), "wipesync");
        assert!(parse_remove_method("wip").is_err());
        assert!(parse_remove_method("unknown").is_err());
    }

    #[test]
    fn test_parse_shred_size_gnu_syntax() {
        let valid = [
            ("+1", 1),
            ("  +0x10", 16),
            ("010", 8),
            ("K", 1024),
            ("1c", 1),
            ("1b", 512),
            ("1B", 1024),
            ("1kB", 1000),
            ("1KiB", 1024),
            ("1KD", 1000),
            ("1M", 1024 * 1024),
            ("0Q", 0),
            ("9223372036854775807", i64::MAX as u64),
            ("0x7fffffffffffffff", i64::MAX as u64),
        ];
        for (value, expected) in valid {
            assert_eq!(parse_shred_size(value), Ok(expected), "value: {value}");
        }
    }

    #[test]
    fn test_parse_shred_size_rejects_gnu_invalid_values() {
        for value in [
            " K",
            "+K",
            "1m",
            "1g",
            "1t",
            "1 ",
            "09",
            "1Q",
            "9223372036854775808",
            "0x8000000000000000",
        ] {
            assert!(parse_shred_size(value).is_err(), "value: {value}");
        }
    }

    #[test]
    fn test_parse_shred_iterations_gnu_syntax() {
        let max_iterations = usize::MAX / std::mem::size_of::<i32>();
        for (value, expected) in [
            ("1", 1usize),
            (" 1", 1usize),
            ("\t+1", 1usize),
            ("01", 1usize),
            ("0", 0usize),
        ] {
            assert_eq!(
                parse_shred_iterations(value),
                Ok(expected),
                "value: {value}"
            );
        }

        let max_value = max_iterations.to_string();
        assert_eq!(parse_shred_iterations(&max_value), Ok(max_iterations));
        assert!(parse_shred_iterations(&(max_iterations + 1).to_string()).is_err());
    }

    #[test]
    fn test_parse_shred_iterations_rejects_trailing_or_signed_whitespace() {
        for value in ["1 ", "+ 1", "-1", "1e2", ""] {
            assert!(parse_shred_iterations(value).is_err(), "value: {value}");
        }
    }

    #[test]
    fn test_gnu_pass_schedule_counts() -> Result<(), io::Error> {
        use tempfile::NamedTempFile;

        let mut source_file = NamedTempFile::new().unwrap();
        source_file.write_all(&[0; 4096]).unwrap();
        source_file.flush().unwrap();

        for (n_passes, expected_random, expected_flipped) in [(25, 3, 0), (48, 4, 22)] {
            let settings = ShredSettings {
                path: Path::new("file"),
                n_passes,
                remove_method: RemoveMethod::None,
                size: None,
                exact: true,
                zero: false,
                verbose: false,
                force: false,
                random_source: Some(source_file.path()),
            };
            let mut random_source = RandomSource::new(Some(source_file.path()))?;
            let sequence = shred_pass_sequence(&settings, &mut random_source)?;
            let random_count = sequence
                .iter()
                .filter(|pass| matches!(pass, PassType::Random))
                .count();
            let flipped_count = sequence
                .iter()
                .filter(
                    |pass| matches!(pass, PassType::Pattern(pattern) if pattern.flip_sector_bit),
                )
                .count();

            assert_eq!(random_count, expected_random);
            assert_eq!(flipped_count, expected_flipped);
        }
        Ok(())
    }

    #[test]
    fn test_zero_pass_name_uses_six_hex_digits() {
        assert_eq!(pass_name(&PassType::Zero), "000000");
    }

    #[test]
    fn test_small_regular_file_gets_inode_pass_before_rounded_pass() -> CTResult<()> {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        let source = dir.path().join("source");
        fs::write(&target, b"0123456789abcdef").unwrap();
        let mut source_data = vec![b'A'; 16];
        source_data.extend(vec![b'B'; 8192]);
        fs::write(&source, source_data).unwrap();

        shred_main(
            [
                OsString::from("shred"),
                OsString::from("-n1"),
                OsString::from("--random-source"),
                source.into_os_string(),
                target.clone().into_os_string(),
            ]
            .into_iter(),
        )?;

        let data = fs::read(target).unwrap();
        assert!(data.len() > 16);
        assert_eq!(&data[..16], b"BBBBBBBBBBBBBBBB");
        Ok(())
    }

    mod pattern_tests {
        use super::*;

        #[test]
        fn test_bytes_writer_random() -> Result<(), io::Error> {
            let mut source = RandomSource::new(None)?;
            let mut writer = BytesWriter::from_pass_type(&PassType::Random, &mut source);

            let block1 = writer.bytes_for_pass(10)?.to_vec();
            let block2 = writer.bytes_for_pass(10)?.to_vec();
            assert_ne!(block1, block2, "随机块应该不相同");
            assert_eq!(block1.len(), 10, "应该生成请求的长度");

            Ok(())
        }

        #[test]
        fn test_bytes_writer_file_random() -> Result<(), io::Error> {
            use std::io::Write;
            use tempfile::NamedTempFile;

            // 创建临时文件作为随机源
            let mut temp_file = NamedTempFile::new().unwrap();
            let test_data = b"hello world test data for random source";
            temp_file.write_all(test_data).unwrap();
            temp_file.flush().unwrap();

            // 使用文件作为随机源
            let mut source = RandomSource::new(Some(temp_file.path()))?;
            let mut writer = BytesWriter::from_pass_type(&PassType::Random, &mut source);

            let block = writer.bytes_for_pass(10)?;
            assert_eq!(block.len(), 10, "应该生成请求的长度");
            assert_eq!(block, &test_data[..10], "应该从文件读取数据");

            Ok(())
        }

        #[test]
        fn test_bytes_writer_file_random_reports_eof() {
            use tempfile::NamedTempFile;

            let temp_file = NamedTempFile::new().unwrap();
            let mut source = RandomSource::new(Some(temp_file.path())).unwrap();
            let mut writer = BytesWriter::from_pass_type(&PassType::Random, &mut source);

            let error = writer.bytes_for_pass(1).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        }

        #[test]
        fn test_bytes_writer_reuses_random_stream() -> Result<(), io::Error> {
            use tempfile::NamedTempFile;

            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"AAAABBBB").unwrap();
            temp_file.flush().unwrap();
            let mut source = RandomSource::new(Some(temp_file.path()))?;

            let first = {
                let mut writer = BytesWriter::from_pass_type(&PassType::Random, &mut source);
                writer.bytes_for_pass(4)?.to_vec()
            };
            let second = {
                let mut writer = BytesWriter::from_pass_type(&PassType::Random, &mut source);
                writer.bytes_for_pass(4)?.to_vec()
            };

            assert_eq!(first, b"AAAA");
            assert_eq!(second, b"BBBB");
            Ok(())
        }

        #[test]
        fn test_flipped_pattern_changes_each_sector() -> Result<(), io::Error> {
            let mut source = RandomSource::new(None)?;
            let pass = PassType::Pattern(Pattern::from_gnu_type(0x1000));
            let mut writer = BytesWriter::from_pass_type(&pass, &mut source);
            let bytes = writer.bytes_for_pass(513)?;

            assert_eq!(bytes[0], 0x80);
            assert!(bytes[1..512].iter().all(|byte| *byte == 0));
            assert_eq!(bytes[512], 0x80);
            Ok(())
        }

        #[test]
        fn test_bytes_writer_with_random_source_none() -> Result<(), io::Error> {
            // 当没有提供随机源时，应该使用默认随机数生成器
            let mut source = RandomSource::new(None)?;
            let mut writer = BytesWriter::from_pass_type(&PassType::Random, &mut source);

            let block1 = writer.bytes_for_pass(10)?.to_vec();
            let block2 = writer.bytes_for_pass(10)?.to_vec();
            assert_ne!(block1, block2, "随机块应该不相同");
            assert_eq!(block1.len(), 10, "应该生成请求的长度");

            Ok(())
        }
    }

    mod filename_iter_tests {
        use super::*;

        #[test]
        fn test_filename_iter_basic() {
            let iter = ShredFilenameIter::new(2);
            let names: Vec<_> = iter.take(5).collect();

            assert_eq!(names.len(), 5, "应该生成5个名字");
            assert!(names.iter().all(|n| n.len() == 2), "名字长度应该是2");
            assert!(
                names
                    .iter()
                    .all(|n| n.chars().all(|c| SHRED_NAME_CHARSET.contains(&(c as u8)))),
                "应该只使用允许的字符"
            );
        }

        #[test]
        fn test_filename_iter_uniqueness() {
            let iter = ShredFilenameIter::new(1);
            let names: HashSet<_> = iter.collect();

            assert_eq!(
                names.len(),
                SHRED_NAME_CHARSET.len(),
                "单字符名字应该生成所有可能的组合"
            );
        }
    }

    mod remove_method_tests {
        use super::*;

        #[test]
        fn test_do_remove_unlink() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test").unwrap();

            let result = shred_do_remove(&file_path, &file_path, false, RemoveMethod::Unlink);

            assert!(result.is_ok(), "删除应该成功");
            assert!(!file_path.exists(), "文件应该被删除");
        }

        #[test]
        fn test_do_remove_unlink_nested_relative_path() {
            let dir = tempdir().unwrap();
            let subdir = dir.path().join("sub");
            fs::create_dir(&subdir).unwrap();
            let file_path = subdir.join("test.txt");
            fs::write(&file_path, "test").unwrap();

            let result = shred_do_remove(&file_path, &file_path, false, RemoveMethod::Unlink);

            assert!(result.is_ok());
            assert!(!file_path.exists());
        }

        #[test]
        fn test_do_remove_wipe() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test").unwrap();

            let result = shred_do_remove(&file_path, &file_path, false, RemoveMethod::Wipe);

            assert!(result.is_ok(), "删除应该成功");
            assert!(!file_path.exists(), "文件应该被删除");
        }
    }

    mod wipe_file_tests {
        use super::*;

        /// 创建测试用的配置
        fn create_test_settings(path: &Path) -> ShredSettings {
            ShredSettings {
                path,
                n_passes: 3,
                remove_method: RemoveMethod::None,
                size: None,
                exact: false,
                zero: false,
                verbose: false,
                force: false,
                random_source: None,
            }
        }

        #[test]
        fn test_wipe_file_basic() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            let test_data = "test data";
            fs::write(&file_path, test_data).unwrap();
            let original_size = test_data.len();

            let mut settings = create_test_settings(&file_path);
            settings.size = Some(original_size as u64);
            settings.exact = true;

            let result = shred_exec(&settings);

            assert!(result.is_ok(), "擦除应该成功");
            assert!(file_path.exists(), "文件应该还存在");

            let content = fs::read(&file_path).unwrap();
            assert_eq!(content.len(), original_size, "文件大小应该保持不变");
            assert_ne!(content, test_data.as_bytes(), "文件内容应该被修改");
        }

        #[test]
        fn test_wipe_file_nonexistent() {
            let settings = create_test_settings(Path::new("nonexistent.txt"));
            let result = shred_exec(&settings);
            assert!(result.is_err(), "不存在的文件应该返回错误");
        }

        #[test]
        fn test_wipe_file_with_removal() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test data").unwrap();

            let mut settings = create_test_settings(&file_path);
            settings.n_passes = 1;
            settings.remove_method = RemoveMethod::Unlink;

            let result = shred_exec(&settings);

            assert!(result.is_ok(), "擦除应该成功");
            assert!(!file_path.exists(), "文件应该被删除");
        }

        #[cfg(unix)]
        #[test]
        fn test_removal_truncates_other_hard_links() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            let link_path = dir.path().join("link.txt");
            fs::write(&file_path, "secret").unwrap();
            fs::hard_link(&file_path, &link_path).unwrap();

            let mut settings = create_test_settings(&file_path);
            settings.n_passes = 0;
            settings.remove_method = RemoveMethod::Unlink;

            shred_exec(&settings).unwrap();

            assert!(!file_path.exists());
            assert_eq!(fs::metadata(link_path).unwrap().len(), 0);
        }

        #[test]
        fn test_wipe_file_zero() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test data").unwrap();

            let mut settings = create_test_settings(&file_path);
            settings.zero = true;

            let result = shred_exec(&settings);

            assert!(result.is_ok(), "擦除应该成功");
            assert!(file_path.exists(), "文件应该还存在");
        }

        #[test]
        fn test_wipe_empty_file_with_size_and_zero() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "").unwrap();

            let mut settings = create_test_settings(&file_path);
            settings.n_passes = 0;
            settings.size = Some(4);
            settings.exact = true;
            settings.zero = true;

            let result = shred_exec(&settings);

            assert!(result.is_ok(), "指定大小的空文件擦除应该成功");
            assert_eq!(fs::read(&file_path).unwrap(), vec![0; 4]);
        }

        #[test]
        fn test_wipe_file_force() -> CTResult<()> {
            let temp_dir = Builder::new().prefix("shred_test").tempdir().unwrap();
            let file_path = temp_dir.path().join("test_file.txt");

            // 创建一个只读文件
            let mut file = File::create(&file_path).unwrap();
            file.write_all(b"test content").unwrap();
            drop(file);

            #[cfg(unix)]
            {
                let mut perms = fs::metadata(&file_path).unwrap().permissions();
                perms.set_readonly(true);
                fs::set_permissions(&file_path, perms).unwrap();
            }

            let settings = ShredSettings {
                path: &file_path,
                n_passes: 1,
                remove_method: RemoveMethod::None,
                size: None,
                exact: false,
                zero: false,
                verbose: false,
                force: true,
                random_source: None,
            };

            let result = shred_exec(&settings);
            assert!(result.is_ok(), "强制模式下应该能处理只读文件");

            Ok(())
        }

        #[test]
        fn test_wipe_file_with_random_source() -> CTResult<()> {
            use std::io::Write;
            use tempfile::NamedTempFile;

            let temp_dir = Builder::new().prefix("shred_test").tempdir().unwrap();
            let file_path = temp_dir.path().join("test_file.txt");

            // 创建要被擦除的文件
            let test_content = b"sensitive data to be wiped";
            let mut file = File::create(&file_path).unwrap();
            file.write_all(test_content).unwrap();
            drop(file);

            // 创建随机源文件 - 确保它足够大
            let mut random_source = NamedTempFile::new().unwrap();
            let random_data = b"this is my custom random data source for testing with lots of random bytes and more content to ensure we have enough data for the test case";
            random_source.write_all(random_data).unwrap();
            random_source.flush().unwrap();
            let settings = ShredSettings {
                path: &file_path,
                n_passes: 1,
                remove_method: RemoveMethod::None,
                size: Some(test_content.len() as u64), // 明确指定文件大小
                exact: true,                           // 使用精确大小
                zero: false,
                verbose: false,
                force: false,
                random_source: Some(random_source.path()),
            };

            let result = shred_exec(&settings);
            assert!(result.is_ok(), "使用随机源文件应该成功: {result:?}");

            // 验证文件仍然存在但内容已被覆盖
            assert!(file_path.exists(), "文件应该仍然存在");
            let content = fs::read(&file_path).unwrap();
            assert_eq!(content.len(), test_content.len(), "文件大小应该保持不变");
            assert_ne!(content, test_content, "文件内容应该已被覆盖");

            Ok(())
        }
    }

    mod do_pass_tests {
        use super::*;

        #[test]
        fn test_do_pass_random() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test data").unwrap();

            let mut file = File::create(&file_path).unwrap();
            let mut random_source = RandomSource::new(None).unwrap();
            let result = shred_do_pass(&mut file, &PassType::Random, true, 10, &mut random_source);

            assert!(result.is_ok(), "随机写入应该成功");
            assert_eq!(
                fs::metadata(&file_path).unwrap().len(),
                10,
                "文件大小应该是指定的长度"
            );
        }

        #[test]
        fn test_do_pass_pattern() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test data").unwrap();

            let mut file = File::create(&file_path).unwrap();
            let mut random_source = RandomSource::new(None).unwrap();
            let result = shred_do_pass(
                &mut file,
                &PassType::Pattern(Pattern::from_gnu_type(0xFFF)),
                true,
                5,
                &mut random_source,
            );

            assert!(result.is_ok(), "模式写入应该成功");
            let content = fs::read(&file_path).unwrap();
            assert_eq!(content.len(), 5, "文件大小应该是指定的长度");
            assert!(
                content.iter().all(|&b| b == 0xFF),
                "所有字节应该是指定的模式"
            );
        }
    }

    mod wipe_name_tests {
        use super::*;

        #[test]
        fn test_wipe_name_basic() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test data").unwrap();

            let result = shred_wipe_name(&file_path, false, RemoveMethod::Wipe);

            assert!(result.is_some(), "应该返回新路径");
            let new_path = result.unwrap();
            assert!(new_path.exists(), "新文件应该存在");
            assert_ne!(new_path, file_path, "文件名应该被修改");
            assert!(!file_path.exists(), "原文件不应该存在");

            // 验证新文件名只包含允许的字符
            let new_name = new_path.file_name().unwrap().to_str().unwrap();
            assert!(
                new_name
                    .chars()
                    .all(|c| SHRED_NAME_CHARSET.contains(&(c as u8))),
                "新文件名应该只包含允许的字符"
            );
        }

        #[test]
        fn test_wipe_name_with_sync() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test data").unwrap();

            let result = shred_wipe_name(&file_path, false, RemoveMethod::WipeSync);

            assert!(result.is_some(), "应该返回新路径");
            let new_path = result.unwrap();
            assert!(new_path.exists(), "新文件应该存在");
        }

        #[test]
        fn test_wipe_name_verbose() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test data").unwrap();

            let result = shred_wipe_name(&file_path, true, RemoveMethod::Wipe);

            assert!(result.is_some(), "应该返回新路径");
            let new_path = result.unwrap();
            assert!(new_path.exists(), "新文件应该存在");
        }

        #[test]
        fn test_wipe_name_decreasing_length() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("longname.txt");
            fs::write(&file_path, "test data").unwrap();

            let result = shred_wipe_name(&file_path, false, RemoveMethod::Wipe);

            assert!(result.is_some(), "应该返回新路径");
            let new_path = result.unwrap();
            let new_name = new_path.file_name().unwrap().to_str().unwrap();
            assert!(
                new_name.len() <= "longname.txt".len(),
                "新文件名应该不长于原文件名"
            );
        }

        #[test]
        fn test_wipe_name_collision_handling() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            let collision_path = dir.path().join("0"); // 可能的目标名字

            fs::write(&file_path, "test data").unwrap();
            fs::write(&collision_path, "existing file").unwrap();

            let result = shred_wipe_name(&file_path, false, RemoveMethod::Wipe);

            assert!(result.is_some(), "应该返回新路径");
            let new_path = result.unwrap();
            assert_ne!(new_path, collision_path, "应该避免文件名冲突");
            assert!(collision_path.exists(), "不应该覆盖已存在的文件");
        }
    }
}
