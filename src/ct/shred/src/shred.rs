/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
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

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_parse_size::parse_size_u64;
use ctcore::{
    ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_show_error, ct_show_if_err,
};
#[cfg(unix)]
use libc::S_IWUSR;
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, Write};
#[cfg(unix)]
use std::os::unix::prelude::PermissionsExt;
use std::path::{Path, PathBuf};

const SHRED_ABOUT: &str = ct_help_about!("shred.md");
const SHRED_USAGE: &str = ct_help_usage!("shred.md");
const SHRED_AFTER_HELP: &str = ct_help_section!("after help", "shred.md");

pub mod shred_options {
    pub const SHRED_FORCE: &str = "force";
    pub const SHRED_FILE: &str = "file";
    pub const SHRED_ITERATIONS: &str = "iterations";
    pub const SHRED_SIZE: &str = "size";
    pub const SHRED_WIPESYNC: &str = "u";
    pub const SHRED_REMOVE: &str = "remove";
    pub const SHRED_VERBOSE: &str = "verbose";
    pub const SHRED_EXACT: &str = "exact";
    pub const SHRED_ZERO: &str = "zero";

    pub mod shred_remove {
        pub const SHRED_UNLINK: &str = "unlink";
        pub const SHRED_WIPE: &str = "wipe";
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

/// Used to generate blocks of bytes of size <= BLOCK_SIZE based on either a give pattern
/// or randomness
// The lint warns about a large difference because StdRng is big, but the buffers are much
// larger anyway, so it's fine.
#[allow(clippy::large_enum_variant)]
enum BytesWriter {
    Random {
        rng: StdRng,
        buffer: [u8; SHRED_BLOCK_SIZE],
    },
    // To write patterns we only write to the buffer once. To be able to do
    // this, we need to extend the buffer with 2 bytes. We can then easily
    // obtain a buffer starting with any character of the pattern that we
    // want with an offset of either 0, 1 or 2.
    //
    // For example, if we have the pattern ABC, but we want to write a block
    // of BLOCK_SIZE starting with B, we just pick the slice [1..BLOCK_SIZE+1]
    // This means that we only have to fill the buffer once and can just reuse
    // it afterwards.
    Pattern {
        offset: usize,
        buffer: [u8; SHRED_PATTERN_BUFFER_SIZE],
    },
}

impl BytesWriter {
    fn from_pass_type(pass: &PassType) -> Self {
        match pass {
            // 创建随机数据生成器
            PassType::Random => Self::Random {
                rng: StdRng::from_entropy(),
                buffer: [0; SHRED_BLOCK_SIZE],
            },
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
                Self::Pattern { offset: 0, buffer }
            }
        }
    }

    fn bytes_for_pass(&mut self, size: usize) -> &[u8] {
        match self {
            // 生成随机数据
            Self::Random { rng, buffer } => {
                let bytes = &mut buffer[..size];
                rng.fill(bytes);
                bytes
            }
            // 从预填充的缓冲区获取数据
            Self::Pattern { offset, buffer } => {
                let bytes = &buffer[*offset..size + *offset];
                // 更新偏移量，确保模式正确对齐
                *offset = (*offset + size) % SHRED_PATTERN_LENGTH;
                bytes
            }
        }
    }
}

/// 文件擦除的配置选项
struct ShredSettings<'a> {
    path_str: &'a str,
    n_passes: usize,
    remove_method: RemoveMethod,
    size: Option<u64>,
    exact: bool,
    zero: bool,
    verbose: bool,
    force: bool,
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
            })
            .collect();

        Ok(settings)
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    shred_main(args)
}
pub fn shred_main(args: impl ctcore::Args) -> CTResult<()> {
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
            .help("change permissions to allow writing if necessary")
            .action(ArgAction::SetTrue),
        Arg::new(shred_options::SHRED_ITERATIONS)
            .long(shred_options::SHRED_ITERATIONS)
            .short('n')
            .help("overwrite N times instead of the default (3)")
            .value_name("NUMBER")
            .default_value("3"),
        Arg::new(shred_options::SHRED_SIZE)
            .long(shred_options::SHRED_SIZE)
            .short('s')
            .value_name("N")
            .help("shred this many bytes (suffixes like K, M, G accepted)"),
        Arg::new(shred_options::SHRED_WIPESYNC)
            .short('u')
            .help("deallocate and remove file after overwriting")
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
            .help("like -u but give control on HOW to delete;  See below")
            .action(ArgAction::Set),
        Arg::new(shred_options::SHRED_VERBOSE)
            .long(shred_options::SHRED_VERBOSE)
            .short('v')
            .help("show progress")
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
            .help("add a final overwrite with zeros to hide shredding")
            .action(ArgAction::SetTrue),
        // Positional arguments
        Arg::new(shred_options::SHRED_FILE)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(SHRED_ABOUT)
        .after_help(SHRED_AFTER_HELP)
        .override_usage(ct_format_usage(SHRED_USAGE))
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
            shred_do_pass(&mut file, &pass_type, settings.exact, size)
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
///
/// # 返回值
/// 成功返回 Ok(())，失败返回 IO 错误
fn shred_do_pass(
    file: &mut File,
    pass_type: &PassType,
    exact: bool,
    file_size: u64,
) -> Result<(), io::Error> {
    // 重置文件指针到开始位置
    file.rewind()?;

    let mut writer = BytesWriter::from_pass_type(pass_type);

    // 按块写入数据
    for _ in 0..(file_size / SHRED_BLOCK_SIZE as u64) {
        let block = writer.bytes_for_pass(SHRED_BLOCK_SIZE);
        file.write_all(block)?;
    }

    // 处理剩余字节
    let bytes_left = (file_size % SHRED_BLOCK_SIZE as u64) as usize;
    if bytes_left > 0 {
        let size = if exact { bytes_left } else { SHRED_BLOCK_SIZE };
        let block = writer.bytes_for_pass(size);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    mod pattern_tests {
        use super::*;

        #[test]
        fn test_bytes_writer_random() {
            let mut writer = BytesWriter::Random {
                rng: StdRng::from_entropy(),
                buffer: [0; SHRED_BLOCK_SIZE],
            };

            let block1 = writer.bytes_for_pass(10).to_vec();
            let block2 = writer.bytes_for_pass(10).to_vec();
            assert_ne!(block1, block2, "随机块应该不相同");
            assert_eq!(block1.len(), 10, "应该生成请求的长度");
        }

        #[test]
        fn test_bytes_writer_pattern() {
            let pattern = Pattern::Single(0xAA);
            let mut writer = BytesWriter::from_pass_type(&PassType::Pattern(pattern));

            let block = writer.bytes_for_pass(10);
            assert_eq!(block.len(), 10, "应该生成请求的长度");
            assert!(block.iter().all(|&b| b == 0xAA), "应该全是指定的模式");
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
}
