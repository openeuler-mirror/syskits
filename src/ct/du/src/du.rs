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

use chrono::{DateTime, Local};
use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};
use ctcore::ct_display::{ct_print_verbatim, Quotable};
use ctcore::ct_error::{set_ct_exit_code, CTError, CTResult, CtSimpleError, FromIo};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_parse_glob;
use ctcore::ct_parse_size::{parse_size_u64, ParseSizeError};
use ctcore::{
    ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_show, ct_show_error,
    ct_show_warning,
};
use glob::Pattern;
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::fmt::Display;
#[cfg(not(windows))]
use std::fs::Metadata;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
#[cfg(not(windows))]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, UNIX_EPOCH};
#[cfg(windows)]
use windows_sys::Win32::Foundation::HANDLE;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, FileStandardInfo, GetFileInformationByHandleEx, FILE_ID_128, FILE_ID_INFO,
    FILE_STANDARD_INFO,
};

mod opt_flags {
    pub const HELP: &str = "help";
    pub const NULL: &str = "0";
    pub const ALL: &str = "all";
    pub const APPARENT_SIZE: &str = "apparent-size";
    pub const BLOCK_SIZE: &str = "block-size";
    pub const BYTES: &str = "b";
    pub const TOTAL: &str = "c";
    pub const MAX_DEPTH: &str = "d";
    pub const HUMAN_READABLE: &str = "h";
    pub const BLOCK_SIZE_1K: &str = "k";
    pub const COUNT_LINKS: &str = "l";
    pub const BLOCK_SIZE_1M: &str = "m";
    pub const SEPARATE_DIRS: &str = "S";
    pub const SUMMARIZE: &str = "s";
    pub const THRESHOLD: &str = "threshold";
    pub const SI: &str = "si";
    pub const TIME: &str = "time";
    pub const TIME_STYLE: &str = "time-style";
    pub const ONE_FILE_SYSTEM: &str = "one-file-system";
    pub const DEREFERENCE: &str = "dereference";
    pub const DEREFERENCE_ARGS: &str = "dereference-args";
    pub const NO_DEREFERENCE: &str = "no-dereference";
    pub const INODES: &str = "inodes";
    pub const EXCLUDE: &str = "exclude";
    pub const EXCLUDE_FROM: &str = "exclude-from";
    pub const FILES0_FROM: &str = "files0-from";
    pub const VERBOSE: &str = "verbose";
    pub const FILE: &str = "FILE";
}

const DU_ABOUT: &str = ct_help_about!("du.md");
const AFTER_HELP: &str = ct_help_section!("after help", "du.md");
const DU_USAGE: &str = ct_help_usage!("du.md");

// TODO: Support Z & Y (currently limited by size of u64)
const UNITS: [(char, u32); 6] = [('E', 6), ('P', 5), ('T', 4), ('G', 3), ('M', 2), ('K', 1)];

struct DuTraversalOptions {
    all: bool,
    separate_dirs: bool,
    one_file_system: bool,
    dereference: DuDeref,
    count_links: bool,
    verbose: bool,
    excludes: Vec<Pattern>,
}

struct DuStatPrinter {
    total: bool,
    inodes: bool,
    max_depth: Option<usize>,
    threshold: Option<DuThreshold>,
    apparent_size: bool,
    size_format: DuSizeFormat,
    time: Option<DuTime>,
    time_format: String,
    line_ending: CtLineEnding,
    summarize: bool,
}

#[derive(PartialEq, Clone)]
enum DuDeref {
    All,
    Args(Vec<PathBuf>),
    None,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DuTime {
    Accessed,
    Modified,
    Created,
}

#[derive(Clone, Debug, PartialEq)]
enum DuSizeFormat {
    Human(u64),
    BlockSize(u64),
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct DuFileInfo {
    file_id: u128,
    dev_id: u64,
}

struct DuStat {
    path: PathBuf,
    is_dir: bool,
    size: u64,
    blocks: u64,
    inodes: u64,
    inode: Option<DuFileInfo>,
    created: Option<u64>,
    accessed: u64,
    modified: u64,
}

impl DuStat {
    /**
     * 创建一个新的DuStat实例。
     *
     * 这个函数会根据提供的路径和遍历选项来获取文件或目录的统计信息。
     *
     * @param path 指向要分析的文件或目录的路径。
     * @param options 包含遍历选项，如是否跟随符号链接。
     * @return std::io::Result<Self> 返回一个包含文件或目录统计信息的DuStat实例，如果发生错误则返回错误信息。
     */
    fn new(du_path: &Path, du_opts: &DuTraversalOptions) -> std::io::Result<Self> {
        // 根据遍历选项决定是否取消符号链接
        let should_dereference = match &du_opts.dereference {
            DuDeref::All => true,
            DuDeref::Args(paths) => paths.contains(&du_path.to_path_buf()),
            DuDeref::None => false,
        };

        // 根据是否取消符号链接来获取元数据
        let metadata = if should_dereference {
            fs::metadata(du_path)
        } else {
            fs::symlink_metadata(du_path)
        }?;

        // 非linux系统下的文件信息处理
        #[cfg(not(windows))]
        {
            let file_info = DuFileInfo {
                file_id: metadata.ino() as u128,
                dev_id: metadata.dev(),
            };

            // 构建并返回DuStat实例
            Ok(Self {
                path: du_path.to_path_buf(),
                is_dir: metadata.is_dir(),
                size: if du_path.is_dir() { 0 } else { metadata.len() },
                blocks: metadata.blocks(),
                inodes: 1,
                inode: Some(file_info),
                created: du_birth_u64(&metadata),
                accessed: metadata.atime() as u64,
                modified: metadata.mtime() as u64,
            })
        }

        // linux系统下的文件信息处理
        #[cfg(windows)]
        {
            let size_on_disk = get_size_on_disk(du_path);
            let file_info = get_file_info(du_path);

            // 构建并返回DuStat实例
            Ok(Self {
                path: du_path.to_path_buf(),
                is_dir: metadata.is_dir(),
                size: if du_path.is_dir() { 0 } else { metadata.len() },
                blocks: size_on_disk / 1024 * 2,
                inodes: 1,
                inode: file_info,
                created: windows_creation_time_to_unix_time(metadata.creation_time()),
                accessed: windows_time_to_unix_time(metadata.last_access_time()),
                modified: windows_time_to_unix_time(metadata.last_write_time()),
            })
        }
    }
}
#[cfg(windows)]
/**
 * 将类似于Linux的文件系统时间戳转换为UNIX时间戳。
 *
 * 此函数用于将Windows时间（自1601年1月1日以来的100纳秒间隔数）转换为UNIX时间戳（自1970年1月1日以来的秒数）。
 * 如果底层文件系统不支持访问时间，函数将返回0。
 *
 */
fn windows_time_to_unix_time(win_time: u64) -> u64 {
    (win_time / 10_000_000).saturating_sub(11_644_473_600)
}

#[cfg(windows)]
/**
 * 将类似于Linux的文件创建时间戳转换为UNIX时间戳。
 *
 * 此函数与`windows_time_to_unix_time`相似，但用于转换文件的创建时间。
 * 如果底层文件系统不支持创建时间，函数将返回`None`。
 *
 */
fn windows_creation_time_to_unix_time(win_time: u64) -> Option<u64> {
    (win_time / 10_000_000).checked_sub(11_644_473_600)
}

#[cfg(not(windows))]
fn du_birth_u64(met_data: &Metadata) -> Option<u64> {
    met_data
        .created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|e| e.as_secs())
}

// 此函数用于获取指定路径的文件或目录在磁盘上占用的空间大小（以字节为单位）。
// 注意：此功能仅在类似Linux的平台上可用。
#[cfg(windows)]
fn get_size_on_disk(file_path: &Path) -> u64 {
    let mut size_on_disk = 0;

    // 打开文件以获取其信息，如果打开失败（例如路径是目录），则直接返回0。
    let file = match fs::File::open(file_path) {
        Ok(file) => file,
        Err(_) => return size_on_disk, // 目录的打开会失败
    };

    unsafe {
        // 准备接收文件信息的结构体并调用Windows API获取信息。
        let mut file_info: FILE_STANDARD_INFO = core::mem::zeroed();
        let file_info_ptr: *mut FILE_STANDARD_INFO = &mut file_info;

        let success = GetFileInformationByHandleEx(
            file.as_raw_handle() as HANDLE,
            FileStandardInfo,
            file_info_ptr as _,
            std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
        );

        // 成功获取信息则将占用空间大小赋值给size_on_disk。
        if success != 0 {
            size_on_disk = file_info.AllocationSize as u64;
        }
    }

    size_on_disk
}

// 此函数用于获取指定路径的文件的特定信息，包括文件ID和设备ID。
// 注意：此功能仅在类似Linux的平台上可用。
#[cfg(windows)]
fn get_file_info(file_path: &Path) -> Option<DuFileInfo> {
    let mut result = None;

    // 尝试打开文件以获取其详细信息。
    let file = match fs::File::open(file_path) {
        Ok(file) => file,
        Err(_) => return result,
    };

    unsafe {
        // 准备接收文件ID信息的结构体并调用Windows API获取信息。
        let mut file_info: FILE_ID_INFO = core::mem::zeroed();
        let file_info_ptr: *mut FILE_ID_INFO = &mut file_info;

        let success = GetFileInformationByHandleEx(
            file.as_raw_handle() as HANDLE,
            FileIdInfo,
            file_info_ptr as _,
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        );

        // 如果获取成功，将信息包装成DuFileInfo类型并存入result中。
        if success != 0 {
            result = Some(DuFileInfo {
                file_id: std::mem::transmute::<FILE_ID_128, u128>(file_info.FileId),
                dev_id: file_info.VolumeSerialNumber,
            });
        }
    }

    result
}

fn du_read_block_size(option: Option<&str>) -> CTResult<u64> {
    if let Some(s) = option {
        parse_size_u64(s)
            .map_err(|e| CtSimpleError::new(1, format_error_message(&e, s, opt_flags::BLOCK_SIZE)))
    } else {
        for env_var in ["DU_BLOCK_SIZE", "BLOCK_SIZE", "BLOCKSIZE"] {
            if let Ok(env_size) = env::var(env_var) {
                if let Ok(v) = parse_size_u64(&env_size) {
                    return Ok(v);
                }
            }
        }
        if env::var("POSIXLY_CORRECT").is_ok() {
            Ok(512)
        } else {
            Ok(1024)
        }
    }
}

/**
 *估算给定路径的磁盘使用空间。
 *
 *`du`函数遍历指定的文件或目录，统计其大小以及所有子目录的大小，并通过`print_tx`通道发送统计结果。
 *如果指定了选项，如排除列表、仅统计链接等，这些选项将影响统计过程。
 *
 *参数:
 *- `mut my_stat`: 一个`DuStat`结构体，包含当前正在处理的文件或目录的信息，如路径、大小、是否为目录等。
 *- `options`: 一个`DuTraversalOptions`结构体，包含遍历文件系统时使用的选项，如是否排除某些模式的文件、是否仅在一个文件系统内统计等。
 *- `depth`: 当前处理文件或目录的深度，用于实现递归遍历。
 *- `seen_inodes`: 一个`HashSet`，用于记录已经统计过的文件或目录的inode，以避免重复统计硬链接。
 *- `print_tx`: 一个`mpsc::Sender`通道的一半，用于向主线程发送统计结果或错误信息。
 *
 *返回值:
 *- `Result<DuStat, Box<mpsc::SendError<CTResult<StatPrintInfo>>>>`: 如果成功，返回更新后的`DuStat`结构体；如果出错，返回一个包含错误信息的`Box`。
 */
#[allow(clippy::cognitive_complexity)]
fn du(
    mut state: DuStat,
    du_opts: &DuTraversalOptions,
    du_depth: usize,
    du_seen_inodes: &mut HashSet<DuFileInfo>,
    du_print_tx: &mpsc::Sender<CTResult<StatPrintInfo>>,
) -> Result<DuStat, Box<mpsc::SendError<CTResult<StatPrintInfo>>>> {
    // 如果当前项是目录，开始遍历其内容
    if state.is_dir {
        // 尝试读取目录内容
        let read = match fs::read_dir(&state.path) {
            Ok(read) => read,
            Err(e) => {
                // 发送读取目录失败的错误信息
                du_print_tx.send(Err(e.map_err_context(|| {
                    format!("cannot read directory {}", state.path.quote())
                })))?;
                return Ok(state);
            }
        };

        'file_loop: for f in read {
            match f {
                // 统计当前文件或目录的信息
                Ok(entry) => {
                    match DuStat::new(&entry.path(), du_opts) {
                        Ok(this_stat) => {
                            // 检查是否应忽略当前项，根据排除列表和其他选项
                            for pattern in &du_opts.excludes {
                                // 查看同时具有短路径和长路径的所有模式
                                // 如果我们有命令 'du foo' 并且要排除搜索 'foo/bar'
                                // 我们需要完整的路径
                                if pattern.matches(&this_stat.path.to_string_lossy())
                                    || pattern.matches(&entry.file_name().into_string().unwrap())
                                {
                                    // 如果目录被忽略，则提前退出
                                    if du_opts.verbose {
                                        println!("{} ignored", &this_stat.path.quote());
                                    }
                                    // 转至下一个文件
                                    continue 'file_loop;
                                }
                            }

                            // 检查是否已统计过当前项的inode，如果是，只更新链接计数
                            if let Some(inode) = this_stat.inode {
                                if du_seen_inodes.contains(&inode) {
                                    if du_opts.count_links {
                                        state.inodes += 1;
                                    }
                                    continue;
                                }
                                du_seen_inodes.insert(inode);
                            }
                            // 递归统计子目录，或更新当前目录的统计信息
                            if this_stat.is_dir {
                                // 检查是否跨越了不同的文件系统
                                if du_opts.one_file_system {
                                    if let (Some(this_inode), Some(my_inode)) =
                                        (this_stat.inode, state.inode)
                                    {
                                        if this_inode.dev_id != my_inode.dev_id {
                                            continue;
                                        }
                                    }
                                }

                                // 合并子目录的统计信息到当前目录
                                let this_stat = du(
                                    this_stat,
                                    du_opts,
                                    du_depth + 1,
                                    du_seen_inodes,
                                    du_print_tx,
                                )?;

                                if !du_opts.separate_dirs {
                                    state.size += this_stat.size;
                                    state.blocks += this_stat.blocks;
                                    state.inodes += this_stat.inodes;
                                }
                                // 发送统计结果
                                du_print_tx.send(Ok(StatPrintInfo {
                                    stat: this_stat,
                                    depth: du_depth + 1,
                                }))?;
                            } else {
                                // 更新文件的统计信息
                                state.size += this_stat.size;
                                state.blocks += this_stat.blocks;
                                state.inodes += 1;
                                // 如果选项为统计所有文件，发送该文件的统计信息
                                if du_opts.all {
                                    du_print_tx.send(Ok(StatPrintInfo {
                                        stat: this_stat,
                                        depth: du_depth + 1,
                                    }))?;
                                }
                            }
                        }
                        // 发送统计失败的错误信息
                        Err(e) => du_print_tx.send(Err(e.map_err_context(|| {
                            format!("cannot access {}", entry.path().quote())
                        })))?,
                    }
                }
                // 发送读取文件错误的信息
                Err(error) => du_print_tx.send(Err(error.into()))?,
            }
        }
    }

    Ok(state)
}

#[derive(Debug)]
enum DuError {
    InvalidMaxDepthArg(String),
    SummarizeDepthConflict(String),
    InvalidTimeStyleArg(String),
    InvalidTimeArg,
    InvalidGlob(String),
}

impl Display for DuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMaxDepthArg(s) => write!(f, "invalid maximum depth {}", s.quote()),
            Self::SummarizeDepthConflict(s) => {
                write!(
                    f,
                    "summarizing conflicts with --max-depth={}",
                    s.maybe_quote()
                )
            }
            Self::InvalidTimeStyleArg(s) => write!(
                f,
                "invalid argument {} for 'time style'
Valid arguments are:
- 'full-iso'
- 'long-iso'
- 'iso'
Try '{} --help' for more information.",
                s.quote(),
                ctcore::ct_execute_phrase()
            ),
            Self::InvalidTimeArg => write!(
                f,
                "'birth' and 'creation' arguments for --time are not supported on this platform.",
            ),
            Self::InvalidGlob(s) => write!(f, "Invalid exclude syntax: {s}"),
        }
    }
}

impl Error for DuError {}

impl CTError for DuError {
    fn code(&self) -> i32 {
        match self {
            Self::InvalidMaxDepthArg(_)
            | Self::SummarizeDepthConflict(_)
            | Self::InvalidTimeStyleArg(_)
            | Self::InvalidTimeArg
            | Self::InvalidGlob(_) => 1,
        }
    }
}

/**
 * 将文件内容读取到一个String类型的vector中。
 *
 * 参数:
 *  - filename: 实现了AsRef<Path>的类型，通常是一个字符串，代表要读取的文件路径。
 *
 * 返回值:
 *  - Vec<String>: 包含文件中每一行内容的vector。
 */
fn du_file_as_vec(file_name: impl AsRef<Path>) -> Vec<String> {
    // 打开指定的文件，如果文件不存在则抛出异常。
    let filename = File::open(file_name).expect("no such file");
    // 创建一个缓冲读取器以提高读取效率。
    let buffer = BufReader::new(filename);

    // 读取文件的每一行，并将它们收集到一个vector中。
    // 如果某一行读取失败，则抛出异常。
    buffer
        .lines()
        .map(|l| l.expect("Could not parse line"))
        .collect()
}

/**
 * 根据命令行提供的 --exclude-from 和/或 --exclude 参数，构建一个忽略文件的 globset 列表。
 *
 */
fn du_build_exclude_patterns(args_match: &ArgMatches) -> CTResult<Vec<Pattern>> {
    // 从 --exclude-from 参数中获取文件路径，并尝试将其内容作为排除模式。
    let exclude_from_iterator = args_match
        .get_many::<String>(opt_flags::EXCLUDE_FROM)
        .unwrap_or_default()
        .flat_map(du_file_as_vec);

    // 从 --exclude 参数中获取排除模式的字符串列表。
    let excludes_iterator = args_match
        .get_many::<String>(opt_flags::EXCLUDE)
        .unwrap_or_default()
        .cloned();

    // 准备存储排除模式的向量。
    let mut exclude_patterns = Vec::new();
    for f in excludes_iterator.chain(exclude_from_iterator) {
        // 如果启用了详细模式，打印正在添加的排除模式。
        if args_match.get_flag(opt_flags::VERBOSE) {
            println!("adding {:?} to the exclude list ", &f);
        }
        // 尝试将排除模式字符串转换为 glob 模式，失败则返回错误。
        match ct_parse_glob::ct_from_str(&f) {
            Ok(glob) => exclude_patterns.push(glob),
            Err(err) => return Err(DuError::InvalidGlob(err.to_string()).into()),
        }
    }
    // 返回构建好的排除模式向量。
    Ok(exclude_patterns)
}

struct StatPrintInfo {
    stat: DuStat,
    depth: usize,
}

impl DuStatPrinter {
    /**
     * 根据提供的文件状态（stat）和当前对象的配置（是否考虑 inode 或明显大小），选择并返回一个代表文件大小的值。
     *
     */
    fn du_choose_size(&self, du_stat: &DuStat) -> u64 {
        if self.inodes {
            // 如果考虑 inode，则返回 inode 的数量
            du_stat.inodes
        } else if self.apparent_size {
            // 如果考虑明显大小，则返回文件的大小
            du_stat.size
        } else {
            // 如果不考虑 inode 也不考虑明显大小，则计算并返回分配给文件的块数量（以 512 字节为单位）
            du_stat.blocks * 512
        }
    }

    /**
     * 打印统计信息。
     *
     * 此函数通过接收 `mpsc::Receiver` 中的 `StatPrintInfo` 消息来统计并打印相关信息。
     * 它会根据消息中的 `Stat` 结构体以及一系列条件（如阈值、最大深度和是否总结）来决定是否打印每个统计项。
     * 在循环结束时，如果设置了 `total`，会打印总数。
     *
     */
    fn du_print_stats(&self, rx_msg: &mpsc::Receiver<CTResult<StatPrintInfo>>) -> CTResult<()> {
        let mut grand_total = 0; // 初始化总统计数值。

        loop {
            let received = rx_msg.recv(); // 尝试接收一个统计信息。

            match received {
                Ok(message) => match message {
                    Ok(stat_info) => {
                        let size = self.du_choose_size(&stat_info.stat); // 根据状态选择合适的大小表示。

                        if stat_info.depth == 0 {
                            grand_total += size; // 如果统计深度为0，则累加到总统计数。
                        }

                        // 只有当不被阈值排除、深度不超过最大深度且（如果不是总结模式或当前是顶层深度）时，才打印统计信息。
                        if !self
                            .threshold
                            .map_or(false, |threshold| threshold.should_exclude(size))
                            && self
                                .max_depth
                                .map_or(true, |max_depth| stat_info.depth <= max_depth)
                            && (!self.summarize || stat_info.depth == 0)
                        {
                            self.du_print_stat(&stat_info.stat, size)?;
                        }
                    }
                    Err(e) => ct_show!(e), // 处理接收错误。
                },
                Err(_) => break, // 如果接收通道被关闭，则退出循环。
            }
        }

        // 如果启用了总结模式，打印总数。
        if self.total {
            print!("{}\ttotal", self.du_convert_size(grand_total));
            print!("{}", self.line_ending);
        }

        Ok(())
    }

    /**
     * 根据指定的大小格式将大小单位转换为可读的字符串格式。
     *
     */
    fn du_convert_size(&self, size: u64) -> String {
        // 如果当前设置为显示iNode大小，则直接返回字节大小的字符串形式
        if self.inodes {
            return size.to_string();
        }
        // 根据大小格式化选项进行不同的大小转换
        match self.size_format {
            DuSizeFormat::Human(multiplier) => {
                // 如果大小为0，直接返回"0"
                if size == 0 {
                    return "0".to_string();
                }
                // 遍历单位列表，找到最合适的单位进行转换
                for &(unit, power) in &UNITS {
                    let limit = multiplier.pow(power);
                    // 如果当前大小超过这个单位的上限，就使用这个单位进行转换
                    if size >= limit {
                        return format!("{:.1}{}", (size as f64) / (limit as f64), unit);
                    }
                }
                // 如果没有超过任何已知单位的上限，就以字节为单位显示
                format!("{size}B")
            }
            DuSizeFormat::BlockSize(block_size) => {
                // 根据块大小格式化选项，将大小转换为对应的块数，并返回块数的字符串形式
                du_div_ceil(size, block_size).to_string()
            }
        }
    }

    /**
     * 打印关于文件或目录的统计信息。
     *
     */
    fn du_print_stat(&self, du_stat: &DuStat, size: u64) -> CTResult<()> {
        // 如果定义了时间格式，则格式化并打印时间
        if let Some(time) = self.time {
            let seconds = du_get_time_secs(time, du_stat)?;
            let du_time = DateTime::<Local>::from(UNIX_EPOCH + Duration::from_secs(seconds));
            let time_string = du_time.format(&self.time_format).to_string();
            // 格式化并打印文件大小和时间
            print!("{}\t{}\t", self.du_convert_size(size), time_string);
        } else {
            // 未定义时间格式时，只格式化并打印文件大小
            print!("{}\t", self.du_convert_size(size));
        }

        // 打印文件或目录的路径
        ct_print_verbatim(&du_stat.path).unwrap();
        print!("{}", self.line_ending);

        Ok(())
    }
}
/**
 * 计算a除以b后的向上取整结果。
 *
 * 此函数为a除以b然后向上取整的实现。它特别优化了当`b`是常数，
 * 尤其是2的幂时的情况。一旦`u64::div_ceil`稳定，此实现可以被替换。
 *
 */
pub fn du_div_ceil(val_1: u64, val_2: u64) -> u64 {
    // 加上除数减1，然后除以除数，是为了向上取整。
    (val_1 + val_2 - 1) / val_2
}

/**
 * 从给定的文件名或标准输入读取文件。
 *
 * 如果文件名是"-"，则从标准输入读取。如果文件名是目录，则返回错误。
 * 尝试打开文件，如果文件不存在或打开时发生其他错误，则返回相应的错误。
 *
 * 文件名通过零字节分隔。读取的路径被添加到结果向量中，重复的路径被忽略。
 * 如果遇到零字节分隔的空路径，则视为错误。
 */
fn du_read_files_from(filename: &str) -> Result<Vec<PathBuf>, std::io::Error> {
    // 根据文件名选择读取来源：标准输入或文件。
    let reader: Box<dyn BufRead> = if filename == "-" {
        Box::new(BufReader::new(std::io::stdin()))
    } else {
        // 检查文件名是否指向一个目录。
        let path = PathBuf::from(filename);
        if path.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{}: read error: Is a directory", filename),
            ));
        }

        // 尝试打开文件并处理文件不存在的错误。
        match File::open(filename) {
            Ok(file) => Box::new(BufReader::new(file)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!(
                        "cannot open '{}' for reading: No such file or directory",
                        filename
                    ),
                ))
            }
            Err(e) => return Err(e),
        }
    };

    let mut paths_buf = Vec::new();

    // 遍历读取的内容，根据零字节分隔的路径进行处理。
    for (i, line) in reader.split(b'\0').enumerate() {
        let path = line?;

        // 空路径被视为错误。
        if path.is_empty() {
            let line_number = i + 1;
            ct_show_error!("{filename}:{line_number}: invalid zero-length file name");
            set_ct_exit_code(1);
        } else {
            // 添加非空路径到结果向量，忽略重复的路径。
            let p = PathBuf::from(String::from_utf8_lossy(&path).to_string());
            if !paths_buf.contains(&p) {
                paths_buf.push(p);
            }
        }
    }

    Ok(paths_buf)
}

#[ctcore::main]
#[allow(clippy::cognitive_complexity)]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    du_main(args).map(|_| ())
}

pub fn du_main(args: impl ctcore::Args) -> CTResult<()> {
    // 从命令行参数中解析匹配项
    let args_match = ct_app().try_get_matches_from(args)?;

    // 解析是否需要汇总信息
    let is_summarize = args_match.get_flag(opt_flags::SUMMARIZE);

    let du_max_depth = du_get_max_depth(&args_match, is_summarize)?;

    // 处理输入文件列表
    let files_path = if let Some(file_from) = args_match.get_one::<String>(opt_flags::FILES0_FROM) {
        // 从文件中读取文件列表，处理特殊值 "-" 表示标准输入
        if file_from == "-" && args_match.get_one::<String>(opt_flags::FILE).is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "extra operand {}\nfile operands cannot be combined with --files0-from",
                    args_match
                        .get_one::<String>(opt_flags::FILE)
                        .unwrap()
                        .quote()
                ),
            )
            .into());
        }

        du_read_files_from(file_from)?
    } else {
        // 直接从命令行参数获取文件列表
        match args_match.get_one::<String>(opt_flags::FILE) {
            Some(_) => args_match
                .get_many::<String>(opt_flags::FILE)
                .unwrap()
                .map(PathBuf::from)
                .collect(),
            None => vec![PathBuf::from(".")], // 默认使用当前目录
        }
    };

    let du_time = du_get_time(&args_match);

    let du_size_format = du_get_size_format(&args_match)?;

    // 解析遍历选项
    let du_traversal_options = DuTraversalOptions {
        all: args_match.get_flag(opt_flags::ALL),
        separate_dirs: args_match.get_flag(opt_flags::SEPARATE_DIRS),
        one_file_system: args_match.get_flag(opt_flags::ONE_FILE_SYSTEM),
        dereference: if args_match.get_flag(opt_flags::DEREFERENCE) {
            DuDeref::All
        } else if args_match.get_flag(opt_flags::DEREFERENCE_ARGS) {
            // 根据参数决定是否只对参数进行解引用
            DuDeref::Args(files_path.clone())
        } else {
            DuDeref::None
        },
        count_links: args_match.get_flag(opt_flags::COUNT_LINKS),
        verbose: args_match.get_flag(opt_flags::VERBOSE),
        excludes: du_build_exclude_patterns(&args_match)?, // 构建排除模式列表
    };

    let du_stat_printer = du_get_stat_printer(
        &args_match,
        is_summarize,
        du_max_depth,
        du_time,
        du_size_format,
    )?;

    // 如果同时指定了 --inodes 和 --apparent-size 或 --bytes，给出警告
    if du_stat_printer.inodes
        && (args_match.get_flag(opt_flags::APPARENT_SIZE) || args_match.get_flag(opt_flags::BYTES))
    {
        ct_show_warning!("options --apparent-size and -b are ineffective with --inodes");
    }

    // 使用独立线程进行输出打印，以便在计算仍在进行时能打印完成的结果
    let (print_tx, rx) = mpsc::channel::<CTResult<StatPrintInfo>>();
    let printing_thread = thread::spawn(move || du_stat_printer.du_print_stats(&rx));

    // 遍历文件列表，对每个文件进行统计
    'loop_file: for path in files_path {
        // 如果配置了排除模式，则检查当前路径是否被排除
        if !&du_traversal_options.excludes.is_empty() {
            let path_string = path.to_string_lossy();
            for pattern in &du_traversal_options.excludes {
                if pattern.matches(&path_string) {
                    // 如果目录被排除，则在 verbose 模式下打印信息，并跳过该目录
                    if du_traversal_options.verbose {
                        println!("{} ignored", path_string.quote());
                    }
                    continue 'loop_file;
                }
            }
        }

        // 检查参数提供的路径是否存在
        if let Ok(stat) = DuStat::new(&path, &du_traversal_options) {
            // 从初始路径开始计算磁盘使用情况
            let mut seen_inodes: HashSet<DuFileInfo> = HashSet::new();
            if let Some(inode) = stat.inode {
                seen_inodes.insert(inode);
            }
            let stat = du(stat, &du_traversal_options, 0, &mut seen_inodes, &print_tx)
                .map_err(|e| CtSimpleError::new(1, e.to_string()))?;

            // 发送统计结果以便打印
            print_tx
                .send(Ok(StatPrintInfo { stat, depth: 0 }))
                .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
        } else {
            // 如果无法访问路径，发送错误信息
            print_tx
                .send(Err(CtSimpleError::new(
                    1,
                    format!(
                        "cannot access {}: No such file or directory",
                        path.to_string_lossy().quote()
                    ),
                )))
                .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
        }
    }

    drop(print_tx); // 释放发送者，结束接收者线程

    // 等待打印线程完成
    printing_thread
        .join()
        .map_err(|_| CtSimpleError::new(1, "Printing thread panicked."))??;

    Ok(())
}

fn du_get_stat_printer(
    args_match: &ArgMatches,
    is_summarize: bool,
    du_max_depth: Option<usize>,
    du_time: Option<DuTime>,
    size_format: DuSizeFormat,
) -> Result<DuStatPrinter, Box<dyn CTError>> {
    // 构建统计信息打印器
    let stat_printer = DuStatPrinter {
        max_depth: du_max_depth,
        size_format,
        summarize: is_summarize,
        total: args_match.get_flag(opt_flags::TOTAL),
        inodes: args_match.get_flag(opt_flags::INODES),
        threshold: args_match
            .get_one::<String>(opt_flags::THRESHOLD)
            .map(|s| {
                DuThreshold::from_str(s).map_err(|e| {
                    CtSimpleError::new(1, format_error_message(&e, s, opt_flags::THRESHOLD))
                })
            })
            .transpose()?,
        apparent_size: args_match.get_flag(opt_flags::APPARENT_SIZE)
            || args_match.get_flag(opt_flags::BYTES),
        time: du_time,
        time_format: du_parse_time_style(
            args_match
                .get_one::<String>("time-style")
                .map(|s| s.as_str()),
        )?
        .to_string(),
        line_ending: CtLineEnding::from_zero_flag(args_match.get_flag(opt_flags::NULL)),
    };
    Ok(stat_printer)
}

fn du_get_size_format(args_match: &ArgMatches) -> Result<DuSizeFormat, Box<dyn CTError>> {
    // 解析大小格式化选项
    let size_format = if args_match.get_flag(opt_flags::HUMAN_READABLE) {
        DuSizeFormat::Human(1024)
    } else if args_match.get_flag(opt_flags::SI) {
        DuSizeFormat::Human(1000)
    } else if args_match.get_flag(opt_flags::BYTES) {
        DuSizeFormat::BlockSize(1)
    } else if args_match.get_flag(opt_flags::BLOCK_SIZE_1K) {
        DuSizeFormat::BlockSize(1024)
    } else if args_match.get_flag(opt_flags::BLOCK_SIZE_1M) {
        DuSizeFormat::BlockSize(1024 * 1024)
    } else {
        DuSizeFormat::BlockSize(du_read_block_size(
            args_match
                .get_one::<String>(opt_flags::BLOCK_SIZE)
                .map(AsRef::as_ref),
        )?)
    };
    Ok(size_format)
}

fn du_get_time(args_match: &ArgMatches) -> Option<DuTime> {
    // 解析显示时间类型选项
    let time = args_match.contains_id(opt_flags::TIME).then(|| {
        match args_match
            .get_one::<String>(opt_flags::TIME)
            .map(AsRef::as_ref)
        {
            None | Some("ctime" | "status") => DuTime::Modified,
            Some("access" | "atime" | "use") => DuTime::Accessed,
            Some("birth" | "creation") => DuTime::Created,
            _ => unreachable!("should be caught by clap"),
        }
    });
    time
}

fn du_get_max_depth(
    args_match: &ArgMatches,
    summarize: bool,
) -> Result<Option<usize>, Box<dyn CTError>> {
    // 解析最大深度
    let du_max_depth = du_parse_depth(
        args_match
            .get_one::<String>(opt_flags::MAX_DEPTH)
            .map(|s| s.as_str()),
        summarize,
    )?;
    Ok(du_max_depth)
}

fn du_get_time_secs(du_time: DuTime, du_stat: &DuStat) -> Result<u64, DuError> {
    match du_time {
        DuTime::Modified => Ok(du_stat.modified),
        DuTime::Accessed => Ok(du_stat.accessed),
        DuTime::Created => du_stat.created.ok_or(DuError::InvalidTimeArg),
    }
}

fn du_parse_time_style(option: Option<&str>) -> CTResult<&str> {
    match option {
        Some(s) => match s {
            "full-iso" => Ok("%Y-%m-%d %H:%M:%S.%f %z"),
            "long-iso" => Ok("%Y-%m-%d %H:%M"),
            "iso" => Ok("%Y-%m-%d"),
            _ => Err(DuError::InvalidTimeStyleArg(s.into()).into()),
        },
        None => Ok("%Y-%m-%d %H:%M"),
    }
}

fn du_parse_depth(du_max_depth_str: Option<&str>, summarize: bool) -> CTResult<Option<usize>> {
    let du_max_depth = du_max_depth_str
        .as_ref()
        .and_then(|s| s.parse::<usize>().ok());
    match (du_max_depth_str, du_max_depth) {
        (Some(s), _) if summarize => Err(DuError::SummarizeDepthConflict(s.into()).into()),
        (Some(s), None) => Err(DuError::InvalidMaxDepthArg(s.into()).into()),
        (Some(_), Some(_)) | (None, _) => Ok(du_max_depth),
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = DU_ABOUT;
    let usage_description = ct_format_usage(DU_USAGE);
    let args = vec![
        Arg::new(opt_flags::HELP)
            .long(opt_flags::HELP)
            .help("Print help information.")
            .action(ArgAction::Help),

        Arg::new(opt_flags::ALL)
            .short('a')
            .long(opt_flags::ALL)
            .help("write counts for all files, not just directories")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::APPARENT_SIZE)
            .long(opt_flags::APPARENT_SIZE)
            .help(
                "print apparent sizes, rather than disk usage \
                although the apparent size is usually smaller, it may be larger due to holes \
                in ('sparse') files, internal fragmentation, indirect blocks, and the like"
            )
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::BLOCK_SIZE)
            .short('B')
            .long(opt_flags::BLOCK_SIZE)
            .value_name("SIZE")
            .help(
                "scale sizes by SIZE before printing them. \
                E.g., '-BM' prints sizes in units of 1,048,576 bytes. See SIZE format below."
            ),

        Arg::new(opt_flags::BYTES)
            .short('b')
            .long("bytes")
            .help("equivalent to '--apparent-size --block-size=1'")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::TOTAL)
            .long("total")
            .short('c')
            .help("produce a grand total")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::MAX_DEPTH)
            .short('d')
            .long("max-depth")
            .value_name("N")
            .help(
                "print the total for a directory (or file, with --all) \
                only if it is N or fewer levels below the command \
                line argument;  --max-depth=0 is the same as --summarize"
            ),

        Arg::new(opt_flags::HUMAN_READABLE)
            .long("human-readable")
            .short('h')
            .help("print sizes in human readable format (e.g., 1K 234M 2G)")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::INODES)
            .long(opt_flags::INODES)
            .help(
                "list inode usage information instead of block usage like --block-size=1K"
            )
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::BLOCK_SIZE_1K)
            .short('k')
            .help("like --block-size=1K")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::COUNT_LINKS)
            .short('l')
            .long("count-links")
            .help("count sizes many times if hard linked")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::DEREFERENCE)
            .short('L')
            .long(opt_flags::DEREFERENCE)
            .help("follow all symbolic links")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::DEREFERENCE_ARGS)
            .short('D')
            .visible_short_alias('H')
            .long(opt_flags::DEREFERENCE_ARGS)
            .help("follow only symlinks that are listed on the command line")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::NO_DEREFERENCE)
            .short('P')
            .long(opt_flags::NO_DEREFERENCE)
            .help("don't follow any symbolic links (this is the default)")
            .overrides_with(opt_flags::DEREFERENCE)
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::BLOCK_SIZE_1M)
            .short('m')
            .help("like --block-size=1M")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::NULL)
            .short('0')
            .long("null")
            .help("end each output line with 0 byte rather than newline")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::SEPARATE_DIRS)
            .short('S')
            .long("separate-dirs")
            .help("do not include size of subdirectories")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::SUMMARIZE)
            .short('s')
            .long("summarize")
            .help("display only a total for each argument")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::SI)
            .long(opt_flags::SI)
            .help("like -h, but use powers of 1000 not 1024")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::ONE_FILE_SYSTEM)
            .short('x')
            .long(opt_flags::ONE_FILE_SYSTEM)
            .help("skip directories on different file systems")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::THRESHOLD)
            .short('t')
            .long(opt_flags::THRESHOLD)
            .value_name("SIZE")
            .num_args(1)
            .allow_hyphen_values(true)
            .help("exclude entries smaller than SIZE if positive, \
                      or entries greater than SIZE if negative"),

        Arg::new(opt_flags::VERBOSE)
            .short('v')
            .long("verbose")
            .help("verbose mode (option not present in GNU/Coreutils)")
            .action(ArgAction::SetTrue),

        Arg::new(opt_flags::EXCLUDE)
            .long(opt_flags::EXCLUDE)
            .value_name("PATTERN")
            .help("exclude files that match PATTERN")
            .action(ArgAction::Append),

        Arg::new(opt_flags::EXCLUDE_FROM)
            .short('X')
            .long("exclude-from")
            .value_name("FILE")
            .value_hint(clap::ValueHint::FilePath)
            .help("exclude files that match any pattern in FILE")
            .action(ArgAction::Append),

        Arg::new(opt_flags::FILES0_FROM)
            .long("files0-from")
            .value_name("FILE")
            .value_hint(clap::ValueHint::FilePath)
            .help("summarize device usage of the NUL-terminated file names specified in file F; if F is -, then read names from standard input")
            .action(ArgAction::Append),

        Arg::new(opt_flags::TIME)
            .long(opt_flags::TIME)
            .value_name("WORD")
            .require_equals(true)
            .num_args(0..)
            .value_parser(["atime", "access", "use", "ctime", "status", "birth", "creation"])
            .help(
                "show time of the last modification of any file in the \
                directory, or any of its subdirectories. If WORD is given, show time as WORD instead \
                of modification time: atime, access, use, ctime, status, birth or creation"
            ),

        Arg::new(opt_flags::TIME_STYLE)
            .long(opt_flags::TIME_STYLE)
            .value_name("STYLE")
            .help(
                "show times using style STYLE: \
                full-iso, long-iso, iso, +FORMAT FORMAT is interpreted like 'date'"
            ),

        Arg::new(opt_flags::FILE)
            .hide(true)
            .value_hint(clap::ValueHint::AnyPath)
            .action(ArgAction::Append),

    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .after_help(AFTER_HELP)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .args(&args)
}

#[derive(Clone, Copy)]
enum DuThreshold {
    Lower(u64),
    Upper(u64),
}

impl FromStr for DuThreshold {
    type Err = ParseSizeError;

    /**
     * 从一个字符串中解析出阈值。
     *
     * 这个函数会根据字符串`s`的前缀（是否包含'-'或'+'）来决定是解析为上限还是下限阈值。
     * 如果字符串以'-'开头且后续的大小为0，则视为无效输入。
     */
    fn from_str(str: &str) -> Result<Self, Self::Err> {
        // 根据字符串是否以'-'或'+'开头，计算出实际大小字符串的起始位置。
        let str_offset = usize::from(str.starts_with(&['-', '+'][..]));

        // 解析大小。
        let str_size = parse_size_u64(&str[str_offset..])?;

        // 如果字符串以'-'开头，判断解析出的大小是否为0，为0则报错。
        if str.starts_with('-') {
            // 阈值为'-0'时，除了大小为0的条目外排除所有条目。
            if str_size == 0 {
                return Err(ParseSizeError::ParseFailure(str.to_string()));
            }
            Ok(Self::Upper(str_size))
        } else {
            // 字符串不以'-'开头，解析为下限阈值。
            Ok(Self::Lower(str_size))
        }
    }
}

impl DuThreshold {
    fn should_exclude(&self, du_size: u64) -> bool {
        match *self {
            Self::Upper(threshold) => du_size > threshold,
            Self::Lower(threshold) => du_size < threshold,
        }
    }
}

/**
 * 根据给定的解析错误、输入字符串和选项，生成一个格式化的错误消息。
 *
 * 这个函数主要用于处理解析大小时出现的错误，能够根据不同的错误类型生成不同的错误提示，
 * 其错误信息的格式会依据用户选择的GNU's du工具的块大小或阈值标志（-B或--block-size，-t或--threshold）。
 *
 */
fn format_error_message(pase_error: &ParseSizeError, s: &str, opt: &str) -> String {
    // 根据不同的错误类型，生成相应的错误信息
    match pase_error {
        ParseSizeError::InvalidSuffix(_) => {
            // 当错误为无效的后缀时，生成包含无效后缀信息的错误消息
            format!("invalid suffix in --{} argument {}", opt, s.quote())
        }
        ParseSizeError::ParseFailure(_) => {
            // 当错误为解析失败时，生成通用的解析错误消息
            format!("invalid --{} argument {}", opt, s.quote())
        }
        ParseSizeError::SizeTooBig(_) => {
            // 当错误为参数值过大时，生成表示参数值过大的错误消息
            format!("--{} argument {} too large", opt, s.quote())
        }
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    mod test_ct_app {
        use super::*;
        use crate::opt_flags::EXCLUDE_FROM;
        use crate::opt_flags::FILES0_FROM;
        use crate::opt_flags::ONE_FILE_SYSTEM;

        use crate::opt_flags::TIME;
        use crate::opt_flags::TIME_STYLE;

        use crate::opt_flags::ALL;
        use crate::opt_flags::APPARENT_SIZE;
        use crate::opt_flags::BLOCK_SIZE;
        use crate::opt_flags::BLOCK_SIZE_1K;
        use crate::opt_flags::BLOCK_SIZE_1M;
        use crate::opt_flags::BYTES;
        use crate::opt_flags::COUNT_LINKS;
        use crate::opt_flags::DEREFERENCE;
        use crate::opt_flags::DEREFERENCE_ARGS;
        use crate::opt_flags::EXCLUDE;
        use crate::opt_flags::HUMAN_READABLE;
        use crate::opt_flags::INODES;
        use crate::opt_flags::MAX_DEPTH;
        use crate::opt_flags::NO_DEREFERENCE;
        use crate::opt_flags::NULL;
        use crate::opt_flags::SEPARATE_DIRS;
        use crate::opt_flags::SI;
        use crate::opt_flags::SUMMARIZE;
        use crate::opt_flags::THRESHOLD;
        use crate::opt_flags::TOTAL;
        use crate::opt_flags::VERBOSE;

        use clap::error::ErrorKind;
        use std::io::Write;
        use tempfile::Builder;

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
        fn test_ct_app_a() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-a"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(ALL).unwrap());
        }

        #[test]
        fn test_ct_app_all() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--all"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(ALL).unwrap());
        }

        #[test]
        fn test_ct_app_apparent_size() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--apparent-size"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
        }

        #[test]
        fn test_ct_app_all_apparent_size() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--all", "--apparent-size"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert!(matches.get_one::<bool>(ALL).unwrap());
        }

        #[test]
        fn test_ct_app_block_size_1k() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1K"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1K");
        }

        #[test]
        fn test_ct_app_block_size_1m() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1M"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1M");
        }

        #[test]
        fn test_ct_app_block_size_1g() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1g"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1g");
        }

        #[test]
        fn test_ct_app_block_size_1t() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1T"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1T");
        }

        #[test]
        fn test_ct_app_block_size_1p() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1P"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1P");
        }

        #[test]
        fn test_ct_app_block_size_1e() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1E"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1E");
        }

        #[test]
        fn test_ct_app_block_size_1z() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1Z"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1Z");
        }

        #[test]
        fn test_ct_app_block_size_1y() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1Y"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1Y");
        }

        #[test]
        fn test_ct_app_apparent_size_block_size_1k() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1K",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());

            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1K");
        }

        #[test]
        fn test_ct_app_apparent_size_block_size_1m() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1M",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1M");
        }

        #[test]
        fn test_ct_app_apparent_size_block_size_1g() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1g",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1g");
        }

        #[test]
        fn test_ct_app_apparent_size_block_size_1t() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1T",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1T");
        }

        #[test]
        fn test_ct_app_apparent_size_block_size_1p() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1P",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1P");
        }

        #[test]
        fn test_ct_app_apparent_size_block_size_1e() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1E",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1E");
        }

        #[test]
        fn test_ct_app_apparent_size_block_size_1z() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1Z",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1Z");
        }

        #[test]
        fn test_ct_app_apparent_size_block_size_1y() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1Y",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "1Y");
        }

        #[test]
        fn test_ct_app_bk() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "BK"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BK");
        }

        #[test]
        fn test_ct_app_bm() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "BM"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BM");
        }

        #[test]
        fn test_ct_app_bg() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "BG"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BG");
        }

        #[test]
        fn test_ct_app_bt() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "BT"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BT");
        }

        #[test]
        fn test_ct_app_bp() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "BP"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BP");
        }

        #[test]
        fn test_ct_app_be() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "BE"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BE");
        }

        #[test]
        fn test_ct_app_bz() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "BZ"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BZ");
        }

        #[test]
        fn test_ct_app_by() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--block-size", "BY"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BY");
        }

        #[test]
        fn test_ct_app_apparent_size_bk() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "BK",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BK");
        }

        #[test]
        fn test_ct_app_apparent_size_bm() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "BM",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BM");
        }

        #[test]
        fn test_ct_app_apparent_size_bg() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "BG",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BG");
        }

        #[test]
        fn test_ct_app_apparent_size_bt() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "BT",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BT");
        }

        #[test]
        fn test_ct_app_apparent_size_bp() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "BP",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BP");
        }

        #[test]
        fn test_ct_app_apparent_size_be() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "BE",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BE");
        }

        #[test]
        fn test_ct_app_apparent_size_bz() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "BZ",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BZ");
        }

        #[test]
        fn test_ct_app_apparent_size_by() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "BY",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(APPARENT_SIZE).unwrap());
            assert_eq!(matches.get_one::<String>(BLOCK_SIZE).unwrap(), "BY");
        }

        #[test]
        fn test_ct_app_b() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-b"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(BYTES).unwrap());
        }

        #[test]
        fn test_ct_app_bytes() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--bytes"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(BYTES).unwrap());
        }

        #[test]
        fn test_ct_app_total() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-c"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(TOTAL).unwrap());
        }

        #[test]
        fn test_ct_app_total_whole() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--total"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(TOTAL).unwrap());
        }

        #[test]
        fn test_ct_app_block_size_flag_k() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-k"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(BLOCK_SIZE_1K).unwrap());
        }

        #[test]
        fn test_ct_app_block_size_flag_1m() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-m"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(BLOCK_SIZE_1M).unwrap());
        }

        #[test]
        fn test_ct_app_max_depth_zeros() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "0"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"0".to_string()
            );
        }

        #[test]
        fn test_ct_app_max_depth_1() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "1"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"1".to_string()
            );
        }

        #[test]
        fn test_ct_app_max_depth_2() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"2".to_string()
            );
        }

        #[test]
        fn test_ct_app_max_depth_3() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "3"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"3".to_string()
            );
        }

        #[test]
        fn test_ct_app_max_depth_4() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "4"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"4".to_string()
            );
        }

        #[test]
        fn test_ct_app_max_depth_5() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "5"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"5".to_string()
            );
        }

        #[test]
        fn test_ct_app_d_zeros() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-d", "0"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"0".to_string()
            );
        }

        #[test]
        fn test_ct_app_d_1() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-d", "1"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"1".to_string()
            );
        }

        #[test]
        fn test_ct_app_d_2() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-d", "2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"2".to_string()
            );
        }

        #[test]
        fn test_ct_app_d_3() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-d", "3"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"3".to_string()
            );
        }

        #[test]
        fn test_ct_app_d_4() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-d", "4"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"4".to_string()
            );
        }

        #[test]
        fn test_ct_app_d_5() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-d", "5"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(MAX_DEPTH).unwrap(),
                &"5".to_string()
            );
        }

        #[test]
        fn test_ct_app_human_readable() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-h"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(HUMAN_READABLE).unwrap());
        }

        #[test]
        fn test_ct_app_human_readable_whole() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--human-readable"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(HUMAN_READABLE).unwrap());
        }

        #[test]
        fn test_ct_app_inodes() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--inodes"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(INODES).unwrap());
        }

        #[test]
        fn test_ct_app_human_readable_inodes() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--human-readable", "--inodes"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(INODES).unwrap());
            assert!(matches.get_one::<bool>(HUMAN_READABLE).unwrap());
        }

        #[test]
        fn test_ct_app_h_human_readable_inodes() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                dir,
                "-h",
                "--human-readable",
                "--inodes",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());

            assert_eq!(result.unwrap_err().kind(), ErrorKind::ArgumentConflict);
        }

        #[test]
        fn test_ct_app_k() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-k"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert!(matches.get_one::<bool>(BLOCK_SIZE_1K).unwrap());
        }

        #[test]
        fn test_ct_app_count_links() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-l"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(COUNT_LINKS).unwrap());
        }

        #[test]
        fn test_ct_app_count_links_whole() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--count-links"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(COUNT_LINKS).unwrap());
        }

        #[test]
        fn test_ct_app_l_count_links_whole() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-l", "--count-links"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());

            assert_eq!(result.unwrap_err().kind(), ErrorKind::ArgumentConflict);
        }

        #[test]
        fn test_ct_app_dereference() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-L"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(DEREFERENCE).unwrap());
        }

        #[test]
        fn test_ct_app_dereference_whole() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--dereference"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(DEREFERENCE).unwrap());
        }

        #[test]
        fn test_ct_app_dereference_args() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "-D"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(DEREFERENCE_ARGS).unwrap());
        }

        #[test]
        fn test_ct_app_dereference_args_whole() {
            let temp_dir = Builder::new().prefix("tests_ct_app_dir").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = sub_dir_path.to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), dir, "--dereference-args"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(DEREFERENCE_ARGS).unwrap());
        }

    }
}