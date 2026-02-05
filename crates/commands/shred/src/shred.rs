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
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_parse_size::parse_size_u64;
use ctcore::{ct_show_error, ct_show_if_err};
#[cfg(unix)]
use libc::S_IWUSR;
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, Write};
#[cfg(unix)]
use std::os::unix::prelude::PermissionsExt;
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
const SHRED_PATTERN_BUFFER_SIZE: usize = SHRED_BLOCK_SIZE + SHRED_PATTERN_LENGTH - 1;

/// Patterns that appear in order for the passes
///
/// They are all extended to 3 bytes for consistency, even though some could be
/// expressed as single bytes.
const SHRED_PATTERNS: [Pattern; 22] = [
    Pattern::Single(b'\x00'),
    Pattern::Single(b'\xFF'),
    Pattern::Single(b'\x55'),
    Pattern::Single(b'\xAA'),
    Pattern::Multi([b'\x24', b'\x92', b'\x49']),
    Pattern::Multi([b'\x49', b'\x24', b'\x92']),
    Pattern::Multi([b'\x6D', b'\xB6', b'\xDB']),
    Pattern::Multi([b'\x92', b'\x49', b'\x24']),
    Pattern::Multi([b'\xB6', b'\xDB', b'\x6D']),
    Pattern::Multi([b'\xDB', b'\x6D', b'\xB6']),
    Pattern::Single(b'\x11'),
    Pattern::Single(b'\x22'),
    Pattern::Single(b'\x33'),
    Pattern::Single(b'\x44'),
    Pattern::Single(b'\x66'),
    Pattern::Single(b'\x77'),
    Pattern::Single(b'\x88'),
    Pattern::Single(b'\x99'),
    Pattern::Single(b'\xBB'),
    Pattern::Single(b'\xCC'),
    Pattern::Single(b'\xDD'),
    Pattern::Single(b'\xEE'),
];

#[derive(Clone, Copy)]
enum Pattern {
    Single(u8),
    Multi([u8; 3]),
}

enum PassType {
    Pattern(Pattern),
    Random,
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

/// 用于生成擦除数据的写入器
///
/// # 变体说明
/// * `Random` - 生成随机数据
///   - `rng`: 随机数生成器
/// * `FileRandom` - 从文件读取随机数据
///   - `reader`: 文件读取器
///   - `buffer`: 数据缓冲区
/// * `Pattern` - 生成固定模式数据
///   - `offset`: 当前偏移量
///   - `buffer`: 预填充的模式缓冲区
///   
/// # 实现说明
/// 为了提高效率，Pattern 模式使用扩展缓冲区。
/// 通过调整偏移量，可以从任意位置开始获取模式数据，
/// 避免了重复填充缓冲区的开销。
enum BytesWriter {
    Random {
        rng: Box<StdRng>,
        buffer: Box<[u8; SHRED_BLOCK_SIZE]>,
    },
    FileRandom {
        reader: BufReader<File>,
        buffer: Box<[u8; SHRED_BLOCK_SIZE]>,
    },
    Pattern {
        offset: usize,
        buffer: Box<[u8; SHRED_PATTERN_BUFFER_SIZE]>,
    },
}

impl BytesWriter {
    fn from_pass_type_with_random_source(
        pass: &PassType,
        random_source: Option<&str>,
    ) -> Result<Self, io::Error> {
        match pass {
            // 创建随机数据生成器
            PassType::Random => {
                if let Some(source_path) = random_source {
                    // 从文件读取随机数据
                    let file = File::open(source_path)?;
                    let reader = BufReader::new(file);
                    Ok(Self::FileRandom {
                        reader,
                        buffer: Box::new([0; SHRED_BLOCK_SIZE]),
                    })
                } else {
                    // 使用默认随机数生成器
                    Ok(Self::Random {
                        rng: Box::new(StdRng::from_entropy()),
                        buffer: Box::new([0; SHRED_BLOCK_SIZE]),
                    })
                }
            }
            // 创建固定模式生成器
            PassType::Pattern(pattern) => {
                let buffer = match pattern {
                    // 单字节模式：重复填充
                    Pattern::Single(byte) => [*byte; SHRED_PATTERN_BUFFER_SIZE],
                    // 多字节模式：按模式长度重复填充
                    Pattern::Multi(bytes) => {
                        let mut buf = [0; SHRED_PATTERN_BUFFER_SIZE];
                        for chunk in buf.chunks_exact_mut(SHRED_PATTERN_LENGTH) {
                            chunk.copy_from_slice(bytes);
                        }
                        buf
                    }
                };
                Ok(Self::Pattern {
                    offset: 0,
                    buffer: Box::new(buffer),
                })
            }
        }
    }

    fn bytes_for_pass(&mut self, size: usize) -> Result<&[u8], io::Error> {
        match self {
            // 生成随机数据
            Self::Random { rng, buffer } => {
                let bytes = &mut buffer[..size];
                rng.fill(bytes);
                Ok(bytes)
            }
            // 从文件读取随机数据
            Self::FileRandom { reader, buffer } => {
                let bytes = &mut buffer[..size];
                let mut total_read = 0;

                while total_read < size {
                    match reader.read(&mut bytes[total_read..]) {
                        Ok(0) => {
                            // 文件读取到末尾，重新定位到开头
                            reader.seek(std::io::SeekFrom::Start(0))?;
                        }
                        Ok(n) => {
                            total_read += n;
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(bytes)
            }
            // 从预填充的缓冲区获取数据
            Self::Pattern { offset, buffer } => {
                let bytes = &buffer[*offset..size + *offset];
                // 更新偏移量，确保模式正确对齐
                *offset = (*offset + size) % SHRED_PATTERN_LENGTH;
                Ok(bytes)
            }
        }
    }
}

/// 文件擦除的配置选项
///
/// # 字段说明
/// * `path_str` - 要擦除的文件路径
/// * `n_passes` - 覆写次数
/// * `remove_method` - 删除方式（无、直接删除、擦除删除、同步擦除删除）
/// * `size` - 指定的文件大小，None 表示使用原始大小
/// * `exact` - 是否精确匹配文件大小
/// * `zero` - 是否在最后用零填充
/// * `verbose` - 是否显示详细信息
/// * `force` - 是否强制写入（修改文件权限）
/// * `random_source` - 随机数据源文件路径
struct ShredSettings<'a> {
    path_str: &'a str,
    n_passes: usize,
    remove_method: RemoveMethod,
    size: Option<u64>,
    exact: bool,
    zero: bool,
    verbose: bool,
    force: bool,
    random_source: Option<&'a str>,
}

impl<'a> ShredSettings<'a> {
    /// 从命令行参数创建配置
    fn new(matches: &'a clap::ArgMatches) -> CTResult<Vec<Self>> {
        // 获取迭代次数
        let n_passes = match matches.get_one::<String>(shred_options::SHRED_ITERATIONS) {
            Some(s) => s.parse::<usize>().map_err(|_| {
                CtSimpleError::new(1, format!("invalid number of passes: {}", s.quote()))
            })?,
            None => unreachable!(),
        };

        // 获取删除方法
        let remove_method = if matches.get_flag(shred_options::SHRED_WIPESYNC) {
            RemoveMethod::WipeSync
        } else if matches.contains_id(shred_options::SHRED_REMOVE) {
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
        } else {
            RemoveMethod::None
        };

        // 获取文件大小
        let size = matches
            .get_one::<String>(shred_options::SHRED_SIZE)
            .map(|size_str| {
                parse_size_u64(size_str).map_err(|_| {
                    CtSimpleError::new(1, format!("invalid file size: {}", size_str.quote()))
                })
            })
            .transpose()?;

        // 获取其他选项
        let exact = matches.get_flag(shred_options::SHRED_EXACT);
        let zero = matches.get_flag(shred_options::SHRED_ZERO);
        let verbose = matches.get_flag(shred_options::SHRED_VERBOSE);
        let force = matches.get_flag(shred_options::SHRED_FORCE);
        let random_source = matches
            .get_one::<String>(shred_options::SHRED_RANDOM_SOURCE)
            .map(|s| s.as_str());

        // 获取所有文件路径并创建配置
        let settings = matches
            .get_many::<String>(shred_options::SHRED_FILE)
            .unwrap()
            .map(|path| Self {
                path_str: path,
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

pub fn shred_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;

    if !matches.contains_id(shred_options::SHRED_FILE) {
        return Err(CTsageError::new(1, "missing file operand"));
    }

    let settings = ShredSettings::new(&matches)?;

    for setting in settings {
        if setting.n_passes == 0 {
            if let Some(_s) = setting.size {
                File::create(setting.path_str).map_err_context(|| {
                    format!("failed to open {} for writing", setting.path_str.quote())
                })?;
            }
            continue;
        }
        shred_exec(&setting)?;
    }

    Ok(())
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
            .value_parser([
                shred_options::shred_remove::SHRED_UNLINK,
                shred_options::shred_remove::SHRED_WIPE,
                shred_options::shred_remove::SHRED_WIPESYNC,
            ])
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
            .value_hint(clap::ValueHint::FilePath),
        // Positional arguments
        Arg::new(shred_options::SHRED_FILE)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("shred.about"))
        .after_help(t!("shred.after_help"))
        .override_usage(t!("shred.usage"))
        .infer_long_args(true)
        .args(&args)
}

fn pass_name(pass_type: &PassType) -> String {
    match pass_type {
        PassType::Random => String::from("random"),
        PassType::Pattern(Pattern::Single(byte)) => format!("{byte:x}{byte:x}{byte:x}"),
        PassType::Pattern(Pattern::Multi([a, b, c])) => format!("{a:x}{b:x}{c:x}"),
    }
}

/// 执行文件擦除操作
fn shred_exec(settings: &ShredSettings) -> CTResult<()> {
    // 验证文件是否存在且是普通文件
    let path = Path::new(settings.path_str);
    if !path.exists() {
        return Err(CtSimpleError::new(
            1,
            format!("{}: No such file or directory", path.maybe_quote()),
        ));
    }
    if !path.is_file() {
        return Err(CtSimpleError::new(
            1,
            format!("{}: Not a file", path.maybe_quote()),
        ));
    }

    let metadata = fs::metadata(path).map_err_context(String::new)?;

    // 如果需要，设置文件为可写
    if settings.force {
        let mut perms = metadata.permissions();
        #[cfg(unix)]
        {
            if (perms.mode() & (S_IWUSR)) == 0 {
                perms.set_mode(S_IWUSR);
            }
        }
        #[cfg(not(unix))]
        perms.set_readonly(false);
        fs::set_permissions(path, perms).map_err_context(String::new)?;
    }

    // 生成擦除序列
    let mut pass_sequence = Vec::new();
    if metadata.len() != 0 {
        // 根据迭代次数生成擦除序列
        if settings.n_passes <= 3 {
            // 少量迭代时使用随机模式
            for _ in 0..settings.n_passes {
                pass_sequence.push(PassType::Random);
            }
        } else {
            // 多次迭代时混合使用固定模式和随机模式
            let n_full_arrays = settings.n_passes / SHRED_PATTERNS.len();
            let remainder = settings.n_passes % SHRED_PATTERNS.len();

            // 填充完整的模式序列
            for _ in 0..n_full_arrays {
                for p in SHRED_PATTERNS {
                    pass_sequence.push(PassType::Pattern(p));
                }
            }
            // 添加剩余的模式
            for pattern in SHRED_PATTERNS.into_iter().take(remainder) {
                pass_sequence.push(PassType::Pattern(pattern));
            }
            // 随机打乱序列
            let mut rng = rand::thread_rng();
            pass_sequence.shuffle(&mut rng);

            // 插入随机模式
            let n_random = 3 + settings.n_passes / 10;
            for i in 0..n_random {
                pass_sequence[i * (settings.n_passes - 1) / (n_random - 1)] = PassType::Random;
            }
        }

        // 如果需要，添加零填充
        if settings.zero {
            pass_sequence.push(PassType::Pattern(SHRED_PATTERNS[0]));
        }
    }

    // 执行擦除
    let total_passes = pass_sequence.len();
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(false)
        .open(path)
        .map_err_context(|| format!("{}: failed to open for writing", path.maybe_quote()))?;

    let size = settings.size.unwrap_or(metadata.len());

    // 执行每一次擦除
    for (i, pass_type) in pass_sequence.into_iter().enumerate() {
        if settings.verbose {
            let pass_name = pass_name(&pass_type);
            ct_show_error!(
                "{}: pass {:2}/{} ({})...",
                path.maybe_quote(),
                i + 1,
                total_passes,
                pass_name
            );
        }
        ct_show_if_err!(
            shred_do_pass(
                &mut file,
                &pass_type,
                settings.exact,
                size,
                settings.random_source
            )
            .map_err_context(|| format!("{}: File write pass failed", path.maybe_quote()))
        );
    }

    // 如果需要，删除文件
    if settings.remove_method != RemoveMethod::None {
        shred_do_remove(
            path,
            settings.path_str,
            settings.verbose,
            settings.remove_method,
        )
        .map_err_context(|| format!("{}: failed to remove file", path.maybe_quote()))?;
    }
    Ok(())
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
    exact: bool,
    file_size: u64,
    random_source: Option<&str>,
) -> Result<(), io::Error> {
    // 重置文件指针到开始位置
    file.rewind()?;

    let mut writer = BytesWriter::from_pass_type_with_random_source(pass_type, random_source)?;

    // 按块写入数据
    for _ in 0..(file_size / SHRED_BLOCK_SIZE as u64) {
        let block = writer.bytes_for_pass(SHRED_BLOCK_SIZE)?;
        file.write_all(block)?;
    }

    // 处理剩余字节
    let bytes_left = (file_size % SHRED_BLOCK_SIZE as u64) as usize;
    if bytes_left > 0 {
        let size = if exact { bytes_left } else { SHRED_BLOCK_SIZE };
        let block = writer.bytes_for_pass(size)?;
        file.write_all(block)?;
    }

    // 确保数据写入磁盘
    file.sync_data()?;

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
    let file_name_len = orig_path.file_name()?.to_str()?.len();
    let mut last_path = PathBuf::from(orig_path);

    // 从长到短尝试不同长度的文件名
    'outer: for length in (1..=file_name_len).rev() {
        // 尝试该长度的所有可能文件名
        for name in ShredFilenameIter::new(length) {
            let new_path = orig_path.with_file_name(name);

            // 跳过已存在的文件名
            if new_path.exists() {
                continue;
            }

            // 尝试重命名
            match fs::rename(&last_path, &new_path) {
                Ok(()) => {
                    if verbose {
                        ct_show_error!(
                            "{}: renamed to {}",
                            last_path.maybe_quote(),
                            new_path.display()
                        );
                    }

                    // 同步文件系统（如果需要）
                    if remove_method == RemoveMethod::WipeSync {
                        if let Ok(new_file) = OpenOptions::new().write(true).open(&new_path) {
                            let _ = new_file.sync_all();
                        }
                    }

                    last_path = new_path;
                    continue 'outer;
                }
                Err(e) => {
                    ct_show_error!(
                        "{}: Couldn't rename to {}: {}",
                        last_path.maybe_quote(),
                        new_path.quote(),
                        e
                    );
                    return None;
                }
            }
        }
    }

    Some(last_path)
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
    orig_filename: &str,
    verbose: bool,
    remove_method: RemoveMethod,
) -> Result<(), io::Error> {
    if verbose {
        ct_show_error!("{}: removing", orig_filename.maybe_quote());
    }

    // 根据删除方法选择最终路径
    let remove_path = if remove_method == RemoveMethod::Unlink {
        Some(path.with_file_name(orig_filename))
    } else {
        shred_wipe_name(path, verbose, remove_method)
    };

    // 删除文件
    if let Some(rp) = remove_path {
        fs::remove_file(rp)?;
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
                .get_one::<String>(shred_options::SHRED_RANDOM_SOURCE)
                .unwrap(),
            "/dev/urandom",
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
            Some("/dev/urandom"),
            "随机源应该正确设置"
        );
        assert_eq!(setting.n_passes, 2, "迭代次数应该正确设置");
        assert_eq!(setting.path_str, "testfile.txt", "文件路径应该正确设置");

        Ok(())
    }

    mod pattern_tests {
        use super::*;

        #[test]
        fn test_bytes_writer_random() -> Result<(), io::Error> {
            let mut writer = BytesWriter::Random {
                rng: Box::new(StdRng::from_entropy()),
                buffer: Box::new([0; SHRED_BLOCK_SIZE]),
            };

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

            let path = temp_file.path().to_str().unwrap();

            // 使用文件作为随机源
            let mut writer =
                BytesWriter::from_pass_type_with_random_source(&PassType::Random, Some(path))?;

            let block = writer.bytes_for_pass(10)?;
            assert_eq!(block.len(), 10, "应该生成请求的长度");
            assert_eq!(block, &test_data[..10], "应该从文件读取数据");

            Ok(())
        }

        #[test]
        fn test_bytes_writer_with_random_source_none() -> Result<(), io::Error> {
            // 当没有提供随机源时，应该使用默认随机数生成器
            let mut writer =
                BytesWriter::from_pass_type_with_random_source(&PassType::Random, None)?;

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

            let result = shred_do_remove(
                &file_path,
                file_path.to_str().unwrap(),
                false,
                RemoveMethod::Unlink,
            );

            assert!(result.is_ok(), "删除应该成功");
            assert!(!file_path.exists(), "文件应该被删除");
        }

        #[test]
        fn test_do_remove_wipe() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test").unwrap();

            let result = shred_do_remove(
                &file_path,
                file_path.to_str().unwrap(),
                false,
                RemoveMethod::Wipe,
            );

            assert!(result.is_ok(), "删除应该成功");
            assert!(!file_path.exists(), "文件应该被删除");
        }
    }

    mod wipe_file_tests {
        use super::*;

        /// 创建测试用的配置
        fn create_test_settings(path: &str) -> ShredSettings {
            ShredSettings {
                path_str: path,
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

            let mut settings = create_test_settings(file_path.to_str().unwrap());
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
            let settings = create_test_settings("nonexistent.txt");
            let result = shred_exec(&settings);
            assert!(result.is_err(), "不存在的文件应该返回错误");
        }

        #[test]
        fn test_wipe_file_with_removal() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test data").unwrap();

            let mut settings = create_test_settings(file_path.to_str().unwrap());
            settings.n_passes = 1;
            settings.remove_method = RemoveMethod::Unlink;

            let result = shred_exec(&settings);

            assert!(result.is_ok(), "擦除应该成功");
            assert!(!file_path.exists(), "文件应该被删除");
        }

        #[test]
        fn test_wipe_file_zero() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test.txt");
            fs::write(&file_path, "test data").unwrap();

            let mut settings = create_test_settings(file_path.to_str().unwrap());
            settings.zero = true;

            let result = shred_exec(&settings);

            assert!(result.is_ok(), "擦除应该成功");
            assert!(file_path.exists(), "文件应该还存在");
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
                path_str: file_path.to_str().unwrap(),
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
            let random_source_path = random_source.path().to_str().unwrap();

            let settings = ShredSettings {
                path_str: file_path.to_str().unwrap(),
                n_passes: 1,
                remove_method: RemoveMethod::None,
                size: Some(test_content.len() as u64), // 明确指定文件大小
                exact: true,                           // 使用精确大小
                zero: false,
                verbose: false,
                force: false,
                random_source: Some(random_source_path),
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
            let result = shred_do_pass(&mut file, &PassType::Random, true, 10, None);

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
            let result = shred_do_pass(
                &mut file,
                &PassType::Pattern(Pattern::Single(0xFF)),
                true,
                5,
                None,
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
