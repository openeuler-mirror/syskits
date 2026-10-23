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
use chrono::{Local, TimeZone};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{
    CTError, CTResult, CTsageError, CtSimpleError, FromIo, set_ct_exit_code, strip_errno,
};
use ctcore::ct_line_ending::CtLineEnding;
#[cfg(not(unix))]
use ctcore::ct_parse_glob;
use ctcore::ct_parse_size::{ParseSizeError, parse_size_u64};
use ctcore::ct_quoting_style::{CtQuotingStyle, escape_name};
use ctcore::{ct_show, ct_show_error, ct_show_warning};
#[cfg(not(unix))]
use glob::Pattern;
#[cfg(unix)]
use nix::dir::Dir;
#[cfg(unix)]
use nix::fcntl::{AtFlags, OFlag};
#[cfg(unix)]
use nix::sys::stat::{Mode, SFlag, fstatat};
use std::collections::HashSet;
use std::env;
use std::error::Error;
#[cfg(unix)]
use std::ffi::{CStr, CString};
use std::ffi::{OsStr, OsString};
use std::fmt::Display;
#[cfg(not(windows))]
use std::fs::Metadata;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
#[cfg(not(windows))]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(unix)]
use std::os::{fd::AsRawFd, unix::ffi::OsStrExt, unix::ffi::OsStringExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
#[cfg(test)]
use std::time::Duration;
use std::time::UNIX_EPOCH;
use sys_locale::get_locale;
#[cfg(windows)]
use windows_sys::Win32::Foundation::HANDLE;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ID_128, FILE_ID_INFO, FILE_STANDARD_INFO, FileIdInfo, FileStandardInfo,
    GetFileInformationByHandleEx,
};

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn bindtextdomain(
        domainname: *const ctcore::libc::c_char,
        dirname: *const ctcore::libc::c_char,
    ) -> *mut ctcore::libc::c_char;
    fn dgettext(
        domainname: *const ctcore::libc::c_char,
        msgid: *const ctcore::libc::c_char,
    ) -> *mut ctcore::libc::c_char;
}

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

// TODO: Support Z & Y (currently limited by size of u64)
const DU_IEC_BASES: [u128; 7] = [
    1,
    1_024,
    1_048_576,
    1_073_741_824,
    1_099_511_627_776,
    1_125_899_906_842_624,
    1_152_921_504_606_846_976,
];
const DU_SI_BASES: [u128; 7] = [
    1,
    1_000,
    1_000_000,
    1_000_000_000,
    1_000_000_000_000,
    1_000_000_000_000_000,
    1_000_000_000_000_000_000,
];
const DU_IEC_SUFFIXES: [&str; 7] = ["", "K", "M", "G", "T", "P", "E"];
const DU_SI_SUFFIXES: [&str; 7] = ["", "k", "M", "G", "T", "P", "E"];

struct DuTraversalOptions {
    all: bool,
    separate_dirs: bool,
    one_file_system: bool,
    dereference: DuDeref,
    count_links: bool,
    verbose: bool,
    excludes: Vec<DuExcludePattern>,
}

struct DuExcludePattern {
    #[cfg(unix)]
    c_pattern: CString,
    #[cfg(not(unix))]
    pattern: Pattern,
}

struct DuStatPrinter {
    total: bool,
    separate_dirs: bool,
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

const DU_INODES_APPARENT_SIZE_WARNING: &str =
    "options --apparent-size and -b are ineffective with --inodes";
const DU_DIAGNOSTIC_QUOTING_STYLE: CtQuotingStyle = CtQuotingStyle::Shell {
    escape: true,
    always_quote: true,
    show_control: false,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuRow {
    pub kind: String,
    pub path: Option<String>,
    pub label: Option<String>,
    pub depth: usize,
    pub is_dir: bool,
    pub display_size: String,
    pub measured_size: u64,
    pub apparent_size_bytes: u64,
    pub allocated_bytes: u64,
    pub inodes: u64,
    pub time_seconds: Option<i64>,
    pub time_display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuSemantic {
    pub rows: Vec<DuRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
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
    Changed,
    Modified,
    Created,
}

#[derive(Clone, Debug, PartialEq)]
enum DuSizeFormat {
    Human(u64),
    BlockSize(u64),
    BlockSizeWithSuffix {
        block_size: u64,
        suffix: &'static str,
    },
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct DuFileInfo {
    file_id: u128,
    dev_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct DuTimestamp {
    seconds: i64,
    nanoseconds: u32,
}

#[cfg(unix)]
enum DuDirectory {
    Open(Dir),
    Unreadable(io::Error),
}

struct DuStat {
    path: PathBuf,
    is_dir: bool,
    size: u64,
    blocks: u64,
    inodes: u64,
    inode: Option<DuFileInfo>,
    created: Option<DuTimestamp>,
    accessed: DuTimestamp,
    changed: DuTimestamp,
    modified: DuTimestamp,
    #[cfg(unix)]
    directory: Option<DuDirectory>,
}

impl DuStat {
    fn merge_time_from(&mut self, other: &Self) {
        self.accessed = self.accessed.max(other.accessed);
        self.changed = self.changed.max(other.changed);
        self.modified = self.modified.max(other.modified);
        self.created = match (self.created, other.created) {
            (Some(current), Some(other)) => Some(current.max(other)),
            (Some(current), None) => Some(current),
            (None, Some(other)) => Some(other),
            (None, None) => None,
        };
    }

    /**
     * 创建一个新的DuStat实例。
     *
     * 这个函数会根据提供的路径和遍历选项来获取文件或目录的统计信息。
     *
     * @param path 指向要分析的文件或目录的路径。
     * @param options 包含遍历选项，如是否跟随符号链接。
     * @return std::io::Result<Self> 返回一个包含文件或目录统计信息的DuStat实例，如果发生错误则返回错误信息。
     */
    fn new(
        du_path: &Path,
        du_opts: &DuTraversalOptions,
        is_command_line_arg: bool,
    ) -> std::io::Result<Self> {
        // 根据遍历选项决定是否取消符号链接
        let should_dereference = match &du_opts.dereference {
            DuDeref::All => true,
            DuDeref::Args(paths) => is_command_line_arg && paths.contains(&du_path.to_path_buf()),
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
                size: if metadata.is_dir() { 0 } else { metadata.len() },
                blocks: metadata.blocks(),
                inodes: 1,
                inode: Some(file_info),
                created: du_birth_u64(&metadata),
                accessed: DuTimestamp {
                    seconds: metadata.atime(),
                    nanoseconds: metadata.atime_nsec() as u32,
                },
                changed: DuTimestamp {
                    seconds: metadata.ctime(),
                    nanoseconds: metadata.ctime_nsec() as u32,
                },
                modified: DuTimestamp {
                    seconds: metadata.mtime(),
                    nanoseconds: metadata.mtime_nsec() as u32,
                },
                #[cfg(unix)]
                directory: metadata
                    .is_dir()
                    .then(|| du_open_path_directory(du_path, should_dereference)),
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
                size: if metadata.is_dir() { 0 } else { metadata.len() },
                blocks: size_on_disk / 1024 * 2,
                inodes: 1,
                inode: file_info,
                created: windows_creation_time_to_unix_time(metadata.creation_time()).map(
                    |seconds| DuTimestamp {
                        seconds,
                        nanoseconds: windows_time_nanoseconds(metadata.creation_time()),
                    },
                ),
                accessed: DuTimestamp {
                    seconds: windows_time_to_unix_time(metadata.last_access_time()),
                    nanoseconds: windows_time_nanoseconds(metadata.last_access_time()),
                },
                changed: DuTimestamp {
                    seconds: windows_time_to_unix_time(metadata.last_write_time()),
                    nanoseconds: windows_time_nanoseconds(metadata.last_write_time()),
                },
                modified: DuTimestamp {
                    seconds: windows_time_to_unix_time(metadata.last_write_time()),
                    nanoseconds: windows_time_nanoseconds(metadata.last_write_time()),
                },
            })
        }
    }

    #[cfg(unix)]
    fn new_at(
        dir_fd: i32,
        name: &OsStr,
        display_path: PathBuf,
        du_opts: &DuTraversalOptions,
    ) -> io::Result<Self> {
        let should_dereference = matches!(&du_opts.dereference, DuDeref::All);
        let metadata = fstatat(
            Some(dir_fd),
            name,
            if should_dereference {
                AtFlags::empty()
            } else {
                AtFlags::AT_SYMLINK_NOFOLLOW
            },
        )
        .map_err(du_nix_error_to_io)?;
        let is_dir = SFlag::from_bits_truncate(metadata.st_mode).contains(SFlag::S_IFDIR);

        Ok(Self {
            path: display_path,
            is_dir,
            size: if is_dir { 0 } else { metadata.st_size as u64 },
            blocks: metadata.st_blocks as u64,
            inodes: 1,
            inode: Some(DuFileInfo {
                file_id: metadata.st_ino as u128,
                dev_id: metadata.st_dev as u64,
            }),
            created: du_birth_at(dir_fd, name, should_dereference),
            accessed: DuTimestamp {
                seconds: metadata.st_atime,
                nanoseconds: metadata.st_atime_nsec as u32,
            },
            changed: DuTimestamp {
                seconds: metadata.st_ctime,
                nanoseconds: metadata.st_ctime_nsec as u32,
            },
            modified: DuTimestamp {
                seconds: metadata.st_mtime,
                nanoseconds: metadata.st_mtime_nsec as u32,
            },
            directory: is_dir.then(|| du_open_directory_at(dir_fd, name, should_dereference)),
        })
    }
}

#[cfg(unix)]
fn du_directory_flags(follow_symlink: bool) -> OFlag {
    let mut flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC;
    if !follow_symlink {
        flags |= OFlag::O_NOFOLLOW;
    }
    flags
}

#[cfg(unix)]
fn du_open_path_directory(path: &Path, follow_symlink: bool) -> DuDirectory {
    match Dir::open(path, du_directory_flags(follow_symlink), Mode::empty()) {
        Ok(directory) => DuDirectory::Open(directory),
        Err(error) => DuDirectory::Unreadable(du_nix_error_to_io(error)),
    }
}

#[cfg(unix)]
fn du_open_directory_at(dir_fd: i32, name: &OsStr, follow_symlink: bool) -> DuDirectory {
    match Dir::openat(
        Some(dir_fd),
        name,
        du_directory_flags(follow_symlink),
        Mode::empty(),
    ) {
        Ok(directory) => DuDirectory::Open(directory),
        Err(error) => DuDirectory::Unreadable(du_nix_error_to_io(error)),
    }
}

#[cfg(unix)]
fn du_nix_error_to_io(error: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(error as i32)
}

#[cfg(target_os = "linux")]
fn du_birth_at(dir_fd: i32, name: &OsStr, follow_symlink: bool) -> Option<DuTimestamp> {
    let name = CString::new(name.as_bytes()).ok()?;
    let mut statx = std::mem::MaybeUninit::<nix::libc::statx>::zeroed();
    let flags = if follow_symlink {
        0
    } else {
        nix::libc::AT_SYMLINK_NOFOLLOW
    };
    let result = unsafe {
        nix::libc::statx(
            dir_fd,
            name.as_ptr(),
            flags,
            nix::libc::STATX_BTIME,
            statx.as_mut_ptr(),
        )
    };
    if result != 0 {
        return None;
    }

    let statx = unsafe { statx.assume_init() };
    ((statx.stx_mask & nix::libc::STATX_BTIME) != 0 && statx.stx_btime.tv_sec >= 0).then_some(
        DuTimestamp {
            seconds: statx.stx_btime.tv_sec,
            nanoseconds: statx.stx_btime.tv_nsec,
        },
    )
}

#[cfg(all(unix, not(target_os = "linux")))]
fn du_birth_at(_dir_fd: i32, _name: &OsStr, _follow_symlink: bool) -> Option<DuTimestamp> {
    None
}
#[cfg(windows)]
/**
 * 将类似于Linux的文件系统时间戳转换为UNIX时间戳。
 *
 * 此函数用于将Windows时间（自1601年1月1日以来的100纳秒间隔数）转换为UNIX时间戳（自1970年1月1日以来的秒数）。
 * 如果底层文件系统不支持访问时间，函数将返回0。
 *
 */
fn windows_time_to_unix_time(win_time: u64) -> i64 {
    i64::try_from(win_time / 10_000_000)
        .unwrap_or(i64::MAX)
        .saturating_sub(11_644_473_600)
}

#[cfg(windows)]
fn windows_time_nanoseconds(win_time: u64) -> u32 {
    ((win_time % 10_000_000) * 100) as u32
}

#[cfg(windows)]
/**
 * 将类似于Linux的文件创建时间戳转换为UNIX时间戳。
 *
 * 此函数与`windows_time_to_unix_time`相似，但用于转换文件的创建时间。
 * 如果底层文件系统不支持创建时间，函数将返回`None`。
 *
 */
fn windows_creation_time_to_unix_time(win_time: u64) -> Option<i64> {
    i64::try_from(win_time / 10_000_000)
        .ok()?
        .checked_sub(11_644_473_600)
}

#[cfg(not(windows))]
fn du_birth_u64(met_data: &Metadata) -> Option<DuTimestamp> {
    met_data
        .created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| {
            Some(DuTimestamp {
                seconds: i64::try_from(duration.as_secs()).ok()?,
                nanoseconds: duration.subsec_nanos(),
            })
        })
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

#[cfg(test)]
fn du_read_block_size(option: Option<&str>) -> CTResult<u64> {
    if let Some(s) = option {
        du_parse_block_size_value(s)
            .map_err(|e| CtSimpleError::new(1, format_error_message(&e, s, opt_flags::BLOCK_SIZE)))
    } else {
        for env_var in ["DU_BLOCK_SIZE", "BLOCK_SIZE", "BLOCKSIZE"] {
            if let Ok(env_size) = env::var(env_var) {
                if let Ok(v) = du_parse_block_size_value(&env_size) {
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

fn du_parse_block_size_value(s: &str) -> Result<u64, ParseSizeError> {
    let value = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let value = value.strip_prefix('+').unwrap_or(value);
    if value.ends_with('b') {
        return Err(ParseSizeError::InvalidSuffix(s.to_string()));
    }
    let block_size = parse_size_u64(value)?;
    if block_size == 0 {
        Err(ParseSizeError::ParseFailure(format!("{}", s.quote())))
    } else {
        Ok(block_size)
    }
}

fn du_read_block_size_format(option: Option<&str>) -> CTResult<DuSizeFormat> {
    if let Some(s) = option {
        du_parse_block_size_format(s)
            .map_err(|e| CtSimpleError::new(1, format_error_message(&e, s, opt_flags::BLOCK_SIZE)))
    } else {
        for env_var in ["DU_BLOCK_SIZE", "BLOCK_SIZE", "BLOCKSIZE"] {
            if let Ok(env_size) = env::var(env_var) {
                if let Ok(size_format) = du_parse_block_size_format(&env_size) {
                    return Ok(size_format);
                }
            }
        }
        Ok(DuSizeFormat::BlockSize(
            if env::var("POSIXLY_CORRECT").is_ok() {
                512
            } else {
                1024
            },
        ))
    }
}

fn du_parse_block_size_format(s: &str) -> Result<DuSizeFormat, ParseSizeError> {
    match s {
        "human-readable" => return Ok(DuSizeFormat::Human(1024)),
        "si" => return Ok(DuSizeFormat::Human(1000)),
        _ => {}
    }

    let block_size = du_parse_block_size_value(s)?;
    if let Some(suffix) = du_block_size_output_suffix(s) {
        Ok(DuSizeFormat::BlockSizeWithSuffix { block_size, suffix })
    } else {
        Ok(DuSizeFormat::BlockSize(block_size))
    }
}

fn du_block_size_output_suffix(s: &str) -> Option<&'static str> {
    if s.as_bytes()
        .first()
        .is_none_or(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    match s {
        "K" | "k" => Some("K"),
        "KB" | "kB" => Some("kB"),
        "KiB" | "kiB" => Some("KiB"),
        "M" | "m" => Some("M"),
        "MB" | "mB" => Some("MB"),
        "MiB" | "miB" => Some("MiB"),
        "G" | "g" => Some("G"),
        "GB" | "gB" => Some("GB"),
        "GiB" | "giB" => Some("GiB"),
        "T" | "t" => Some("T"),
        "TB" | "tB" => Some("TB"),
        "TiB" | "tiB" => Some("TiB"),
        "P" | "p" => Some("P"),
        "PB" | "pB" => Some("PB"),
        "PiB" | "piB" => Some("PiB"),
        "E" | "e" => Some("E"),
        "EB" | "eB" => Some("EB"),
        "EiB" | "eiB" => Some("EiB"),
        _ => None,
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
        #[cfg(unix)]
        {
            let mut directory = match state.directory.take() {
                Some(DuDirectory::Open(directory)) => directory,
                Some(DuDirectory::Unreadable(error)) => {
                    du_print_tx.send(Err(error.map_err_context(|| {
                        format!("cannot read directory {}", state.path.quote())
                    })))?;
                    return Ok(state);
                }
                None => unreachable!("directories are opened when their metadata is read"),
            };
            let dir_fd = directory.as_raw_fd();

            for entry in directory.iter() {
                match entry {
                    Ok(entry) => {
                        let name = OsStr::from_bytes(entry.file_name().to_bytes());
                        if name == OsStr::new(".") || name == OsStr::new("..") {
                            continue;
                        }
                        let name = name.to_os_string();
                        let entry_path = state.path.join(&name);
                        if du_path_is_excluded(&du_opts.excludes, &entry_path) {
                            if du_opts.verbose {
                                println!("{} ignored", entry_path.quote());
                            }
                            continue;
                        }
                        let this_stat = DuStat::new_at(dir_fd, &name, entry_path.clone(), du_opts);
                        du_process_entry(
                            &mut state,
                            this_stat,
                            &entry_path,
                            du_opts,
                            du_depth,
                            du_seen_inodes,
                            du_print_tx,
                        )?;
                    }
                    Err(error) => du_print_tx.send(Err(du_nix_error_to_io(error).into()))?,
                }
            }
        }

        #[cfg(not(unix))]
        {
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

            for entry in read {
                match entry {
                    Ok(entry) => {
                        let entry_path = entry.path();
                        if du_path_is_excluded(&du_opts.excludes, &entry_path) {
                            if du_opts.verbose {
                                println!("{} ignored", entry_path.quote());
                            }
                            continue;
                        }
                        du_process_entry(
                            &mut state,
                            DuStat::new(&entry_path, du_opts, false),
                            &entry_path,
                            du_opts,
                            du_depth,
                            du_seen_inodes,
                            du_print_tx,
                        )?;
                    }
                    Err(error) => du_print_tx.send(Err(error.into()))?,
                }
            }
        }
    }

    Ok(state)
}

#[allow(clippy::too_many_arguments)]
fn du_process_entry(
    state: &mut DuStat,
    this_stat: io::Result<DuStat>,
    entry_path: &Path,
    du_opts: &DuTraversalOptions,
    du_depth: usize,
    du_seen_inodes: &mut HashSet<DuFileInfo>,
    du_print_tx: &mpsc::Sender<CTResult<StatPrintInfo>>,
) -> Result<(), Box<mpsc::SendError<CTResult<StatPrintInfo>>>> {
    let this_stat = match this_stat {
        Ok(stat) => stat,
        Err(error) => {
            du_print_tx.send(Err(
                error.map_err_context(|| format!("cannot access {}", entry_path.quote()))
            ))?;
            return Ok(());
        }
    };

    // 默认模式下避免重复统计同一 inode；--count-links 时不过滤。
    if !du_opts.count_links || this_stat.is_dir {
        if let Some(inode) = this_stat.inode {
            if du_seen_inodes.contains(&inode) {
                return Ok(());
            }
            du_seen_inodes.insert(inode);
        }
    }

    if this_stat.is_dir {
        if du_opts.one_file_system {
            if let (Some(this_inode), Some(my_inode)) = (this_stat.inode, state.inode) {
                if this_inode.dev_id != my_inode.dev_id {
                    return Ok(());
                }
            }
        }

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
            state.merge_time_from(&this_stat);
        }
        du_print_tx.send(Ok(StatPrintInfo {
            stat: this_stat,
            depth: du_depth + 1,
        }))?;
    } else {
        state.size += this_stat.size;
        state.blocks += this_stat.blocks;
        state.inodes += 1;
        state.merge_time_from(&this_stat);
        if du_opts.all {
            du_print_tx.send(Ok(StatPrintInfo {
                stat: this_stat,
                depth: du_depth + 1,
            }))?;
        }
    }

    Ok(())
}

#[derive(Debug)]
enum DuError {
    InvalidMaxDepthArg(String),
    SummarizeAllConflict,
    SummarizeDepthConflict(String),
    InvalidTimeStyleArg(String),
    InvalidTimeArg,
    InvalidGlob(String),
}

impl Display for DuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMaxDepthArg(s) => write!(f, "invalid maximum depth {}", s.quote()),
            Self::SummarizeAllConflict => {
                write!(f, "cannot both summarize and show all entries")
            }
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
            | Self::SummarizeAllConflict
            | Self::SummarizeDepthConflict(_)
            | Self::InvalidTimeStyleArg(_)
            | Self::InvalidTimeArg
            | Self::InvalidGlob(_) => 1,
        }
    }

    fn usage(&self) -> bool {
        matches!(self, Self::SummarizeAllConflict)
    }
}

#[derive(Debug)]
struct DuClapError {
    code: i32,
    error: clap::Error,
}

impl DuClapError {
    fn new(error: clap::Error) -> Self {
        let code = match error.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => 0,
            _ => 1,
        };
        Self { code, error }
    }
}

impl Display for DuClapError {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = self.error.print();
        Ok(())
    }
}

impl Error for DuClapError {}

impl CTError for DuClapError {
    fn code(&self) -> i32 {
        self.code
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DuLongOptionArgument {
    None,
    Required,
    Optional,
}

fn du_recognized_long_option(candidate: &str) -> Option<(&'static str, DuLongOptionArgument)> {
    if candidate.is_empty() {
        return None;
    }

    let mut matches = [
        ("all", DuLongOptionArgument::None),
        ("apparent-size", DuLongOptionArgument::None),
        ("block-size", DuLongOptionArgument::Required),
        ("bytes", DuLongOptionArgument::None),
        ("count-links", DuLongOptionArgument::None),
        ("dereference", DuLongOptionArgument::None),
        ("dereference-args", DuLongOptionArgument::None),
        ("exclude", DuLongOptionArgument::Required),
        ("exclude-from", DuLongOptionArgument::Required),
        ("files0-from", DuLongOptionArgument::Required),
        ("help", DuLongOptionArgument::None),
        ("human-readable", DuLongOptionArgument::None),
        ("inodes", DuLongOptionArgument::None),
        ("max-depth", DuLongOptionArgument::Required),
        ("no-dereference", DuLongOptionArgument::None),
        ("null", DuLongOptionArgument::None),
        ("one-file-system", DuLongOptionArgument::None),
        ("separate-dirs", DuLongOptionArgument::None),
        ("si", DuLongOptionArgument::None),
        ("summarize", DuLongOptionArgument::None),
        ("threshold", DuLongOptionArgument::Required),
        ("time", DuLongOptionArgument::Optional),
        ("time-style", DuLongOptionArgument::Required),
        ("total", DuLongOptionArgument::None),
        ("verbose", DuLongOptionArgument::None),
        ("version", DuLongOptionArgument::None),
    ]
    .into_iter()
    .filter(|(option, _)| option.starts_with(candidate));
    let option = matches.next()?;
    matches.next().is_none().then_some(option)
}

fn du_long_option_with_unexpected_argument(arg: &str) -> Option<&'static str> {
    let (candidate, _) = arg.strip_prefix("--")?.split_once('=')?;
    let (option, argument) = du_recognized_long_option(candidate)?;
    (argument == DuLongOptionArgument::None).then_some(option)
}

fn du_short_option_consumes_next(arg: &str) -> bool {
    let Some(options) = arg.strip_prefix('-') else {
        return false;
    };

    for (index, option) in options.char_indices() {
        if matches!(option, 'B' | 'X' | 'd' | 't') {
            return index + option.len_utf8() == options.len();
        }
    }
    false
}

fn du_invalid_short_option_byte(arg: &OsStr) -> Option<u8> {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    #[cfg(unix)]
    let bytes = arg.as_bytes();
    #[cfg(not(unix))]
    let bytes = arg.to_string_lossy();
    #[cfg(not(unix))]
    let bytes = bytes.as_bytes();

    for option in bytes.iter().copied().skip(1) {
        match option {
            b'0' | b'D' | b'H' | b'L' | b'P' | b'S' | b'a' | b'b' | b'c' | b'h' | b'k' | b'l'
            | b'm' | b's' | b'v' | b'x' => {}
            b'B' | b'X' | b'd' | b't' => return None,
            invalid => return Some(invalid),
        }
    }
    None
}

fn write_du_invalid_option_diagnostic(
    writer: &mut impl Write,
    utility_name: &str,
    invalid: u8,
) -> io::Result<()> {
    write!(writer, "{utility_name}: invalid option -- '")?;
    writer.write_all(&[invalid])?;
    writeln!(writer, "'")?;
    writeln!(writer, "Try '{utility_name} --help' for more information.")
}

fn du_validate_options(args: &[OsString]) -> CTResult<()> {
    let mut index = 1;
    while index < args.len() {
        let arg_os = &args[index];
        let arg = arg_os.to_string_lossy();
        if arg == "--" {
            break;
        }

        if arg.starts_with("--") {
            if let Some(option) = du_long_option_with_unexpected_argument(&arg) {
                ct_show_error!("option '--{}' doesn't allow an argument", option);
                eprintln!(
                    "Try '{} --help' for more information.",
                    ctcore::ct_help_utility_name()
                );
                return Err(CtSimpleError::new(1, ""));
            }

            if !arg.contains('=')
                && du_recognized_long_option(&arg[2..])
                    .is_some_and(|(_, argument)| argument == DuLongOptionArgument::Required)
            {
                index += 1;
            }
        } else if arg.starts_with('-') && arg != "-" {
            if let Some(invalid) = du_invalid_short_option_byte(arg_os.as_os_str()) {
                let stderr = io::stderr();
                write_du_invalid_option_diagnostic(
                    &mut stderr.lock(),
                    &ctcore::ct_util_name(),
                    invalid,
                )
                .expect("failed to write invalid option diagnostic");
                return Err(CtSimpleError::new(1, ""));
            }
            if du_short_option_consumes_next(&arg) {
                index += 1;
            }
        }

        index += 1;
    }
    Ok(())
}

fn du_last_files0_from(args_match: &ArgMatches) -> Option<&String> {
    args_match
        .get_many::<String>(opt_flags::FILES0_FROM)
        .and_then(|values| values.last())
}

fn du_get_dereference(args_match: &ArgMatches, files_path: &[PathBuf]) -> DuDeref {
    let last_option = [
        opt_flags::DEREFERENCE_ARGS,
        opt_flags::DEREFERENCE,
        opt_flags::NO_DEREFERENCE,
    ]
    .into_iter()
    .filter(|option| {
        args_match.value_source(option) == Some(clap::parser::ValueSource::CommandLine)
    })
    .filter_map(|option| args_match.index_of(option).map(|index| (index, option)))
    .max_by_key(|(index, _)| *index)
    .map(|(_, option)| option);

    match last_option {
        Some(opt_flags::DEREFERENCE) => DuDeref::All,
        Some(opt_flags::DEREFERENCE_ARGS) => DuDeref::Args(files_path.to_vec()),
        Some(opt_flags::NO_DEREFERENCE) | None => DuDeref::None,
        Some(_) => unreachable!("all dereference options are handled"),
    }
}

fn du_apply_posix_option_order(args: &mut Vec<OsString>, enabled: bool) {
    if !enabled {
        return;
    }

    let mut index = 1;
    while index < args.len() {
        let arg = args[index].to_string_lossy();
        if arg == "--" {
            return;
        }
        if arg == "-" || !arg.starts_with('-') {
            args.insert(index, OsString::from("--"));
            return;
        }

        let consumes_next = if let Some(option) = arg.strip_prefix("--") {
            !option.contains('=')
                && du_recognized_long_option(option)
                    .is_some_and(|(_, argument)| argument == DuLongOptionArgument::Required)
        } else {
            du_short_option_consumes_next(&arg)
        };
        index += if consumes_next { 2 } else { 1 };
    }
}

/**
 * 将文件内容按换行符读取到字节vector中。
 *
 * 参数:
 *  - filename: 实现了AsRef<Path>的类型，通常是一个字符串，代表要读取的文件路径。
 *
 * 返回值:
 *  - Vec<Vec<u8>>: 包含文件中每一行原始字节的vector。
 */
fn du_file_as_vec(file_name: impl AsRef<Path>) -> CTResult<Vec<Vec<u8>>> {
    let file_name = file_name.as_ref();
    // 打开指定的文件，失败时按普通命令错误返回，不能 panic 退出 101。
    let filename = File::open(file_name).map_err(|err| {
        CTsageError::new(1, format!("{}: {}", file_name.display(), strip_errno(&err)))
    })?;
    // 创建一个缓冲读取器以提高读取效率。
    let buffer = BufReader::new(filename);

    buffer
        .split(b'\n')
        .map(|line| {
            line.map_err(|err| {
                CTsageError::new(1, format!("{}: {}", file_name.display(), strip_errno(&err)))
            })
        })
        .collect()
}

#[cfg(not(unix))]
fn du_parse_exclude_pattern(pattern: &str) -> Result<Pattern, glob::PatternError> {
    let mut pattern = pattern.to_string();
    loop {
        match ct_parse_glob::ct_from_str(&pattern) {
            Ok(pattern) => return Ok(pattern),
            Err(error) => {
                let mut characters: Vec<_> = pattern.chars().collect();
                if characters.get(error.pos) != Some(&'[')
                    || characters
                        .get(error.pos + 1..)
                        .is_some_and(|suffix| suffix.contains(&']'))
                {
                    return Err(error);
                }
                characters.splice(error.pos..=error.pos, ['[', '[', ']']);
                pattern = characters.into_iter().collect();
            }
        }
    }
}

impl DuExcludePattern {
    fn new(pattern: Vec<u8>) -> Result<Self, String> {
        #[cfg(unix)]
        {
            let c_pattern = CString::new(pattern).map_err(|error| error.to_string())?;
            Ok(Self { c_pattern })
        }
        #[cfg(not(unix))]
        {
            let pattern = String::from_utf8_lossy(&pattern);
            let pattern = du_parse_exclude_pattern(&pattern).map_err(|error| error.to_string())?;
            Ok(Self { pattern })
        }
    }
}

#[cfg(unix)]
fn du_exclude_pattern_matches_bytes(pattern: &DuExcludePattern, path: &[u8]) -> bool {
    CString::new(path).is_ok_and(|path| unsafe {
        ctcore::libc::fnmatch(pattern.c_pattern.as_ptr(), path.as_ptr(), 0) == 0
    })
}

fn du_exclude_pattern_matches_path(pattern: &DuExcludePattern, path: &Path) -> bool {
    #[cfg(unix)]
    {
        let path = path.as_os_str().as_bytes();
        return du_exclude_pattern_matches_bytes(pattern, path)
            || path.iter().enumerate().any(|(index, byte)| {
                *byte == b'/'
                    && path.get(index + 1) != Some(&b'/')
                    && du_exclude_pattern_matches_bytes(pattern, &path[index + 1..])
            });
    }

    #[cfg(not(unix))]
    let path = path.to_string_lossy();
    #[cfg(not(unix))]
    return pattern.pattern.matches(&path)
        || path.match_indices('/').any(|(index, _)| {
            let suffix = &path[index + 1..];
            !suffix.starts_with('/') && pattern.pattern.matches(suffix)
        });
}

fn du_path_is_excluded(excludes: &[DuExcludePattern], path: &Path) -> bool {
    excludes
        .iter()
        .any(|pattern| du_exclude_pattern_matches_path(pattern, path))
}

/**
 * 根据命令行提供的 --exclude-from 和/或 --exclude 参数，构建一个忽略文件的 globset 列表。
 *
 */
fn du_build_exclude_patterns(args_match: &ArgMatches) -> CTResult<Vec<DuExcludePattern>> {
    // 从 --exclude-from 参数中获取文件路径，并尝试将其内容作为排除模式。
    let mut exclude_from_patterns = Vec::new();
    for file_name in args_match
        .get_many::<String>(opt_flags::EXCLUDE_FROM)
        .unwrap_or_default()
    {
        exclude_from_patterns.extend(du_file_as_vec(file_name)?);
    }

    // 从 --exclude 参数中获取排除模式的字符串列表。
    let excludes_iterator = args_match
        .get_many::<String>(opt_flags::EXCLUDE)
        .unwrap_or_default()
        .map(|pattern| pattern.as_bytes().to_vec());

    // 准备存储排除模式的向量。
    let mut exclude_patterns = Vec::new();
    for f in excludes_iterator.chain(exclude_from_patterns) {
        // 如果启用了详细模式，打印正在添加的排除模式。
        if args_match.get_flag(opt_flags::VERBOSE) {
            println!(
                "adding {:?} to the exclude list ",
                String::from_utf8_lossy(&f)
            );
        }
        // 尝试将排除模式字符串转换为 glob 模式，失败则返回错误。
        match DuExcludePattern::new(f) {
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

    fn du_should_print(&self, stat_info: &StatPrintInfo, size: u64) -> bool {
        !self
            .threshold
            .is_some_and(|threshold| threshold.should_exclude(size))
            && self
                .max_depth
                .is_none_or(|max_depth| stat_info.depth <= max_depth)
            && (!self.summarize || stat_info.depth == 0)
    }

    fn du_contributes_to_total(&self, stat_info: &StatPrintInfo) -> bool {
        stat_info.depth == 0 || (self.separate_dirs && stat_info.stat.is_dir)
    }

    fn du_format_time(&self, timestamp: DuTimestamp) -> String {
        let du_time = Local
            .timestamp_opt(timestamp.seconds, timestamp.nanoseconds)
            .single()
            .expect("filesystem timestamp is outside chrono range");
        if let Some(fmt) = self.time_format.strip_prefix('+') {
            du_format_local_time_strftime(timestamp.seconds, timestamp.nanoseconds, fmt)
                .unwrap_or_else(|| du_time.format(fmt).to_string())
        } else {
            du_time.format(&self.time_format).to_string()
        }
    }

    fn du_time_info(&self, du_stat: &DuStat) -> CTResult<Option<(DuTimestamp, String)>> {
        if let Some(time) = self.time {
            let timestamp = du_get_timestamp(time, du_stat)?;
            Ok(Some((timestamp, self.du_format_time(timestamp))))
        } else {
            Ok(None)
        }
    }

    fn du_add_to_total(
        &self,
        stat_info: &StatPrintInfo,
        size: u64,
        grand_total: &mut u64,
        latest_time: &mut Option<DuTimestamp>,
    ) -> CTResult<()> {
        if self.du_contributes_to_total(stat_info) {
            *grand_total += size;
            if let Some(time) = self.time {
                let timestamp = du_get_timestamp(time, &stat_info.stat)?;
                *latest_time = Some(latest_time.map_or(timestamp, |latest| latest.max(timestamp)));
            }
        }
        Ok(())
    }

    fn du_render_stat_prefix(&self, du_stat: &DuStat, size: u64) -> CTResult<String> {
        let mut line = String::new();
        if let Some((_, time_string)) = self.du_time_info(du_stat)? {
            line.push_str(&self.du_convert_size(size));
            line.push('\t');
            line.push_str(&time_string);
            line.push('\t');
        } else {
            line.push_str(&self.du_convert_size(size));
            line.push('\t');
        }
        Ok(line)
    }

    fn du_render_stat_line(&self, du_stat: &DuStat, size: u64) -> CTResult<String> {
        let mut line = self.du_render_stat_prefix(du_stat, size)?;
        line.push_str(&du_stat.path.display().to_string());
        line.push_str(&format!("{}", self.line_ending));
        Ok(line)
    }

    fn du_row_from_stat(&self, du_stat: &DuStat, depth: usize, size: u64) -> CTResult<DuRow> {
        let time_info = self.du_time_info(du_stat)?;
        Ok(DuRow {
            kind: "entry".into(),
            path: Some(du_stat.path.display().to_string()),
            label: None,
            depth,
            is_dir: du_stat.is_dir,
            display_size: self.du_convert_size(size),
            measured_size: size,
            apparent_size_bytes: du_stat.size,
            allocated_bytes: du_stat.blocks * 512,
            inodes: du_stat.inodes,
            time_seconds: time_info.as_ref().map(|(timestamp, _)| timestamp.seconds),
            time_display: time_info.map(|(_, display)| display),
        })
    }

    fn du_total_row(&self, grand_total: u64, latest_time: Option<DuTimestamp>) -> DuRow {
        DuRow {
            kind: "total".into(),
            path: None,
            label: Some(t!("du.total")),
            depth: 0,
            is_dir: false,
            display_size: self.du_convert_size(grand_total),
            measured_size: grand_total,
            apparent_size_bytes: 0,
            allocated_bytes: 0,
            inodes: 0,
            time_seconds: latest_time.map(|timestamp| timestamp.seconds),
            time_display: latest_time.map(|timestamp| self.du_format_time(timestamp)),
        }
    }

    fn du_render_total_line(&self, total_row: &DuRow) -> String {
        let mut line = format!("{}\t", total_row.display_size);
        if let Some(time_display) = &total_row.time_display {
            line.push_str(time_display);
            line.push('\t');
        }
        line.push_str(total_row.label.as_deref().unwrap_or_default());
        line.push_str(&format!("{}", self.line_ending));
        line
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
        let mut latest_time = None;

        loop {
            let received = rx_msg.recv(); // 尝试接收一个统计信息。

            match received {
                Ok(message) => match message {
                    Ok(stat_info) => {
                        let size = self.du_choose_size(&stat_info.stat); // 根据状态选择合适的大小表示。

                        self.du_add_to_total(&stat_info, size, &mut grand_total, &mut latest_time)?;

                        if self.du_should_print(&stat_info, size) {
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
            let total_row = self.du_total_row(grand_total, latest_time);
            print!("{}", self.du_render_total_line(&total_row));
        }

        Ok(())
    }

    /**
     * 根据指定的大小格式将大小单位转换为可读的字符串格式。
     *
     */
    fn du_convert_size(&self, size: u64) -> String {
        // 删除了原本粗暴的 `if self.inodes { return size.to_string(); }`
        // 因为 --inodes 同样需要支持 -h 和 --si 的人类可读格式

        match self.size_format {
            DuSizeFormat::Human(multiplier) => du_human_readable(size, multiplier),
            DuSizeFormat::BlockSize(block_size) => {
                // 根据块大小格式化选项，将大小转换为对应的块数
                if self.inodes {
                    size.to_string() // inode 模式下不受 block_size 除法影响
                } else {
                    du_div_ceil(size, block_size).to_string()
                }
            }
            DuSizeFormat::BlockSizeWithSuffix { block_size, suffix } => {
                if self.inodes {
                    size.to_string()
                } else {
                    format!("{}{}", du_div_ceil(size, block_size), suffix)
                }
            }
        }
    }

    /**
     * 打印关于文件或目录的统计信息。
     *
     */
    fn du_print_stat(&self, du_stat: &DuStat, size: u64) -> CTResult<()> {
        let output = io::stdout();
        let mut stdout = output.lock();
        stdout.write_all(self.du_render_stat_prefix(du_stat, size)?.as_bytes())?;
        du_write_path(&mut stdout, &du_stat.path)?;
        stdout.write_all(format!("{}", self.line_ending).as_bytes())?;
        Ok(())
    }
}

#[cfg(unix)]
fn du_write_path(writer: &mut impl Write, path: &Path) -> io::Result<()> {
    writer.write_all(path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
fn du_write_path(writer: &mut impl Write, path: &Path) -> io::Result<()> {
    write!(writer, "{}", path.display())
}

fn du_inodes_apparent_size_warning(
    args_match: &ArgMatches,
    du_stat_printer: &DuStatPrinter,
) -> Option<&'static str> {
    if du_stat_printer.inodes
        && (args_match.get_flag(opt_flags::APPARENT_SIZE) || args_match.get_flag(opt_flags::BYTES))
    {
        Some(DU_INODES_APPARENT_SIZE_WARNING)
    } else {
        None
    }
}

#[cfg(unix)]
fn du_format_local_time_strftime(seconds: i64, nanoseconds: u32, fmt: &str) -> Option<String> {
    let time = ctcore::libc::time_t::try_from(seconds).ok()?;
    let mut tm: ctcore::libc::tm = unsafe { std::mem::zeroed() };
    let mut expanded_fmt = String::with_capacity(fmt.len());
    let mut format_chars = fmt.chars().peekable();
    while let Some(character) = format_chars.next() {
        if character == '%' {
            match format_chars.peek() {
                Some('N') => {
                    expanded_fmt.push_str(&format!("{nanoseconds:09}"));
                    format_chars.next();
                    continue;
                }
                Some('%') => {
                    expanded_fmt.push_str("%%");
                    format_chars.next();
                    continue;
                }
                _ => {}
            }
        }
        expanded_fmt.push(character);
    }
    let c_format = CString::new(expanded_fmt).ok()?;
    let tm_ptr = unsafe { ctcore::libc::localtime_r(&time as *const _, &mut tm as *mut _) };
    if tm_ptr.is_null() {
        return None;
    }

    let mut capacity = 128usize;
    while capacity <= 8192 {
        let mut buf = vec![0u8; capacity];
        let written = unsafe {
            ctcore::libc::strftime(
                buf.as_mut_ptr() as *mut ctcore::libc::c_char,
                capacity,
                c_format.as_ptr(),
                &tm as *const _,
            )
        };
        if written > 0 {
            buf.truncate(written);
            return String::from_utf8(buf).ok();
        }
        capacity *= 2;
    }
    None
}

#[cfg(not(unix))]
fn du_format_local_time_strftime(_seconds: i64, _nanoseconds: u32, _fmt: &str) -> Option<String> {
    None
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
    val_1.div_ceil(val_2)
}

fn du_human_readable(size: u64, multiplier: u64) -> String {
    let (bases, suffixes) = if multiplier == 1000 {
        (&DU_SI_BASES, &DU_SI_SUFFIXES)
    } else {
        (&DU_IEC_BASES, &DU_IEC_SUFFIXES)
    };
    du_localize_decimal_point(du_to_magnitude_and_suffix(size as u128, bases, suffixes))
}

fn du_init_locale() {
    #[cfg(unix)]
    unsafe {
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr());
    }
    #[cfg(target_os = "linux")]
    unsafe {
        bindtextdomain(c"coreutils".as_ptr(), c"/usr/share/locale".as_ptr());
    }
}

fn du_localize_decimal_point(number: String) -> String {
    #[cfg(unix)]
    unsafe {
        let locale = ctcore::libc::localeconv();
        if !locale.is_null() && !(*locale).decimal_point.is_null() {
            let decimal_point = CStr::from_ptr((*locale).decimal_point).to_string_lossy();
            return du_localize_decimal_point_with(number, &decimal_point);
        }
    }

    number
}

fn du_localize_decimal_point_with(mut number: String, decimal_point: &str) -> String {
    if decimal_point != "." && !decimal_point.is_empty() {
        if let Some(index) = number.find('.') {
            number.replace_range(index..=index, decimal_point);
        }
    }
    number
}

fn du_to_magnitude_and_suffix(n: u128, bases: &[u128], suffixes: &[&'static str]) -> String {
    let mut magnitude = 0usize;
    while magnitude + 1 < bases.len() && bases[magnitude + 1] <= n {
        magnitude += 1;
    }

    if magnitude == 0 {
        return n.to_string();
    }

    let divisor = bases[magnitude];
    let suffix = suffixes[magnitude];
    if n < 10 * divisor {
        let tenths = (n * 10).div_ceil(divisor);
        if tenths < 100 {
            return format!("{}.{}{}", tenths / 10, tenths % 10, suffix);
        }
    }

    let rounded = n.div_ceil(divisor);
    let unit_base = bases[1];
    if rounded == unit_base && magnitude + 1 < bases.len() {
        format!("1.0{}", suffixes[magnitude + 1])
    } else {
        format!("{rounded}{suffix}")
    }
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
        Box::new(BufReader::new(ctcore::ct_io::stdin_reader_box()))
    } else {
        // 检查文件名是否指向一个目录。
        let path = PathBuf::from(filename);
        if path.is_dir() {
            return Err(std::io::Error::other(format!(
                "{filename}: read error: Is a directory"
            )));
        }

        // 尝试打开文件并处理文件不存在的错误。
        match File::open(filename) {
            Ok(file) => Box::new(BufReader::new(file)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(std::io::Error::other(format!(
                    "cannot open '{filename}' for reading: No such file or directory"
                )));
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
            // 【核心修复】：拦截在从 stdin 读取列表时，列表内容出现 "-" 的套娃情况
            if filename == "-" && path == b"-" {
                ct_show_error!(
                    "when reading file names from standard input, no file name of '-' allowed"
                );
                set_ct_exit_code(1);
                continue; // 拒绝将 '-' 加入执行队列，避免后续报 cannot access 的错
            }

            // 添加非空路径到结果向量，忽略重复的路径。
            let p = du_path_from_bytes(path);
            if !paths_buf.contains(&p) {
                paths_buf.push(p);
            }
        }
    }

    Ok(paths_buf)
}

#[cfg(unix)]
fn du_path_from_bytes(path: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(path))
}

#[cfg(not(unix))]
fn du_path_from_bytes(path: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&path).into_owned())
}

fn du_access_error_message(path: &Path, error: &io::Error) -> String {
    let quoted_path = escape_name(path.as_os_str(), &DU_DIAGNOSTIC_QUOTING_STYLE);
    #[cfg(target_os = "linux")]
    if let Some(message) = du_localized_access_error(&quoted_path, error) {
        return message;
    }

    du_format_access_error("cannot access %s", &quoted_path, &strip_errno(error))
}

fn du_format_access_error(template: &str, quoted_path: &str, detail: &str) -> String {
    format!("{}: {detail}", template.replacen("%s", quoted_path, 1))
}

#[cfg(target_os = "linux")]
fn du_localized_access_error(quoted_path: &str, error: &io::Error) -> Option<String> {
    let quoted_path = CString::new(quoted_path).ok()?;
    let errno = error.raw_os_error()?;

    unsafe {
        let template = dgettext(c"coreutils".as_ptr(), c"cannot access %s".as_ptr());
        let detail = ctcore::libc::strerror(errno);
        if template.is_null() || detail.is_null() {
            return None;
        }

        Some(du_format_access_error(
            &CStr::from_ptr(template).to_string_lossy(),
            &quoted_path.to_string_lossy(),
            &CStr::from_ptr(detail).to_string_lossy(),
        ))
    }
}

#[derive(Default)]
pub struct Du;
impl Tool for Du {
    fn name(&self) -> &'static str {
        "du"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        du_main(args.iter().cloned()).map(|_| ())
    }
}

pub fn du_main(args: impl ctcore::Args) -> CTResult<()> {
    du_init_locale();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut args = args.collect::<Vec<_>>();
    du_apply_posix_option_order(&mut args, env::var_os("POSIXLY_CORRECT").is_some());
    du_validate_options(&args)?;
    // 从命令行参数中解析匹配项
    let args_match = ct_app()
        .try_get_matches_from(args)
        .map_err(|error| Box::new(DuClapError::new(error)) as Box<dyn CTError>)?;

    // 解析是否需要汇总信息
    let is_summarize = args_match.get_flag(opt_flags::SUMMARIZE);

    du_check_option_conflicts(&args_match)?;

    let du_max_depth = du_get_max_depth(&args_match, is_summarize)?;
    if let Some(warning) = du_summarize_depth_warning(is_summarize, du_max_depth) {
        ct_show_warning!("{warning}");
    }

    // 处理输入文件列表
    let files_path = if let Some(file_from) = du_last_files0_from(&args_match) {
        // 从文件中读取文件列表，处理特殊值 "-" 表示标准输入
        if args_match.get_one::<PathBuf>(opt_flags::FILE).is_some() {
            return Err(std::io::Error::other(format!(
                "extra operand {}\nfile operands cannot be combined with --files0-from",
                args_match
                    .get_one::<PathBuf>(opt_flags::FILE)
                    .unwrap()
                    .quote()
            ))
            .into());
        }

        du_read_files_from(file_from)?
    } else {
        // 直接从命令行参数获取文件列表
        match args_match.get_one::<PathBuf>(opt_flags::FILE) {
            Some(_) => args_match
                .get_many::<PathBuf>(opt_flags::FILE)
                .unwrap()
                .cloned()
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
        dereference: du_get_dereference(&args_match, &files_path),
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
    if let Some(warning) = du_inodes_apparent_size_warning(&args_match, &du_stat_printer) {
        ct_show_warning!("{warning}");
    }

    // 使用独立线程进行输出打印，以便在计算仍在进行时能打印完成的结果
    let (print_tx, rx) = mpsc::channel::<CTResult<StatPrintInfo>>();
    let printing_thread = thread::spawn(move || du_stat_printer.du_print_stats(&rx));

    // 跨顶层参数共享 inode 去重集合，以匹配 GNU du 在多参数下的行为。
    let mut seen_inodes: HashSet<DuFileInfo> = HashSet::new();

    // 遍历文件列表，对每个文件进行统计
    'loop_file: for path in files_path {
        // 如果配置了排除模式，则检查当前路径是否被排除
        if !&du_traversal_options.excludes.is_empty() {
            let path_string = path.to_string_lossy();
            for pattern in &du_traversal_options.excludes {
                if du_exclude_pattern_matches_path(pattern, &path) {
                    // 如果目录被排除，则在 verbose 模式下打印信息，并跳过该目录
                    if du_traversal_options.verbose {
                        println!("{} ignored", path_string.quote());
                    }
                    continue 'loop_file;
                }
            }
        }
        // --count-links counts each command-line root independently.  Keep
        // directory inode tracking only within one root to prevent cycles.
        if du_traversal_options.count_links {
            seen_inodes.clear();
        }
        // 检查参数提供的路径是否存在
        let stat = match DuStat::new(&path, &du_traversal_options, true) {
            Ok(stat) => stat,
            Err(error) => {
                // 如果无法访问路径，发送保留真实 errno 的错误信息
                print_tx
                    .send(Err(CtSimpleError::new(
                        1,
                        du_access_error_message(&path, &error),
                    )))
                    .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
                continue;
            }
        };
        if let Some(inode) = stat.inode {
            // 【核心修复】：顶层入口也一样，目录必须始终过滤防重
            if !du_traversal_options.count_links || stat.is_dir {
                if seen_inodes.contains(&inode) {
                    continue;
                }
                seen_inodes.insert(inode);
            }
        }

        // 从初始路径开始计算磁盘使用情况
        let stat = du(stat, &du_traversal_options, 0, &mut seen_inodes, &print_tx)
            .map_err(|e| CtSimpleError::new(1, e.to_string()))?;

        // 发送统计结果以便打印
        print_tx
            .send(Ok(StatPrintInfo { stat, depth: 0 }))
            .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
    }

    drop(print_tx); // 释放发送者，结束接收者线程

    // 等待打印线程完成
    printing_thread
        .join()
        .map_err(|_| CtSimpleError::new(1, "Printing thread panicked."))??;

    Ok(())
}

fn du_collect_semantic(
    du_stat_printer: &DuStatPrinter,
    rx_msg: &mpsc::Receiver<CTResult<StatPrintInfo>>,
) -> CTResult<DuSemantic> {
    let mut grand_total = 0u64;
    let mut latest_time = None;
    let mut rows = Vec::new();
    let mut classic_text = String::new();
    let mut stderr_text = String::new();
    let mut exit_code = 0;

    loop {
        let received = rx_msg.recv();
        match received {
            Ok(message) => match message {
                Ok(stat_info) => {
                    let size = du_stat_printer.du_choose_size(&stat_info.stat);
                    du_stat_printer.du_add_to_total(
                        &stat_info,
                        size,
                        &mut grand_total,
                        &mut latest_time,
                    )?;
                    if du_stat_printer.du_should_print(&stat_info, size) {
                        classic_text
                            .push_str(&du_stat_printer.du_render_stat_line(&stat_info.stat, size)?);
                        rows.push(du_stat_printer.du_row_from_stat(
                            &stat_info.stat,
                            stat_info.depth,
                            size,
                        )?);
                    }
                }
                Err(err) => {
                    exit_code = err.code();
                    stderr_text.push_str(&format!("du: {err}\n"));
                }
            },
            Err(_) => break,
        }
    }

    if du_stat_printer.total {
        let total_row = du_stat_printer.du_total_row(grand_total, latest_time);
        classic_text.push_str(&du_stat_printer.du_render_total_line(&total_row));
        rows.push(total_row);
    }

    Ok(DuSemantic {
        rows,
        classic_text,
        stderr_text,
        exit_code,
    })
}

pub fn du_native_semantic(args: impl ctcore::Args) -> CTResult<DuSemantic> {
    du_init_locale();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut args = args.collect::<Vec<_>>();
    du_apply_posix_option_order(&mut args, env::var_os("POSIXLY_CORRECT").is_some());
    let args_match = ct_app().try_get_matches_from(args)?;

    let is_summarize = args_match.get_flag(opt_flags::SUMMARIZE);
    du_check_option_conflicts(&args_match)?;
    let du_max_depth = du_get_max_depth(&args_match, is_summarize)?;
    let summarize_depth_warning = du_summarize_depth_warning(is_summarize, du_max_depth);

    let files_path = if let Some(file_from) = du_last_files0_from(&args_match) {
        if args_match.get_one::<PathBuf>(opt_flags::FILE).is_some() {
            return Err(std::io::Error::other(format!(
                "extra operand {}\nfile operands cannot be combined with --files0-from",
                args_match
                    .get_one::<PathBuf>(opt_flags::FILE)
                    .unwrap()
                    .quote()
            ))
            .into());
        }
        du_read_files_from(file_from)?
    } else {
        match args_match.get_one::<PathBuf>(opt_flags::FILE) {
            Some(_) => args_match
                .get_many::<PathBuf>(opt_flags::FILE)
                .unwrap()
                .cloned()
                .collect(),
            None => vec![PathBuf::from(".")],
        }
    };

    let du_time = du_get_time(&args_match);
    let du_size_format = du_get_size_format(&args_match)?;
    let du_traversal_options = DuTraversalOptions {
        all: args_match.get_flag(opt_flags::ALL),
        separate_dirs: args_match.get_flag(opt_flags::SEPARATE_DIRS),
        one_file_system: args_match.get_flag(opt_flags::ONE_FILE_SYSTEM),
        dereference: du_get_dereference(&args_match, &files_path),
        count_links: args_match.get_flag(opt_flags::COUNT_LINKS),
        verbose: args_match.get_flag(opt_flags::VERBOSE),
        excludes: du_build_exclude_patterns(&args_match)?,
    };

    let du_stat_printer = du_get_stat_printer(
        &args_match,
        is_summarize,
        du_max_depth,
        du_time,
        du_size_format,
    )?;

    let (print_tx, rx) = mpsc::channel::<CTResult<StatPrintInfo>>();
    let mut seen_inodes: HashSet<DuFileInfo> = HashSet::new();

    'loop_file: for path in files_path {
        if !du_traversal_options.excludes.is_empty() {
            for pattern in &du_traversal_options.excludes {
                if du_exclude_pattern_matches_path(pattern, &path) {
                    continue 'loop_file;
                }
            }
        }

        if du_traversal_options.count_links {
            seen_inodes.clear();
        }

        let stat = match DuStat::new(&path, &du_traversal_options, true) {
            Ok(stat) => stat,
            Err(error) => {
                print_tx
                    .send(Err(CtSimpleError::new(
                        1,
                        du_access_error_message(&path, &error),
                    )))
                    .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
                continue;
            }
        };
        if let Some(inode) = stat.inode {
            if !du_traversal_options.count_links || stat.is_dir {
                if seen_inodes.contains(&inode) {
                    continue;
                }
                seen_inodes.insert(inode);
            }
        }

        let stat = du(stat, &du_traversal_options, 0, &mut seen_inodes, &print_tx)
            .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
        print_tx
            .send(Ok(StatPrintInfo { stat, depth: 0 }))
            .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
    }

    drop(print_tx);
    let mut semantic = du_collect_semantic(&du_stat_printer, &rx)?;
    if let Some(warning) = du_inodes_apparent_size_warning(&args_match, &du_stat_printer) {
        semantic
            .stderr_text
            .insert_str(0, &format!("du: warning: {warning}\n"));
    }
    if let Some(warning) = summarize_depth_warning {
        semantic
            .stderr_text
            .insert_str(0, &format!("du: warning: {warning}\n"));
    }
    Ok(semantic)
}

fn du_get_stat_printer(
    args_match: &ArgMatches,
    is_summarize: bool,
    du_max_depth: Option<usize>,
    du_time: Option<DuTime>,
    size_format: DuSizeFormat,
) -> Result<DuStatPrinter, Box<dyn CTError>> {
    let time_format = if du_time.is_some() {
        du_parse_time_style(du_effective_time_style(args_match).as_deref())?.to_string()
    } else {
        String::new()
    };

    // 构建统计信息打印器
    let stat_printer = DuStatPrinter {
        max_depth: du_max_depth,
        size_format,
        summarize: is_summarize,
        total: args_match.get_flag(opt_flags::TOTAL),
        separate_dirs: args_match.get_flag(opt_flags::SEPARATE_DIRS),
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
        time_format,
        line_ending: CtLineEnding::from_zero_flag(args_match.get_flag(opt_flags::NULL)),
    };
    Ok(stat_printer)
}

fn du_effective_time_style(args_match: &ArgMatches) -> Option<String> {
    if let Some(style) = args_match.get_one::<String>(opt_flags::TIME_STYLE) {
        return Some(style.clone());
    }

    let mut style = env::var("TIME_STYLE").ok()?;
    if style == "locale" {
        return Some("long-iso".to_string());
    }
    if style.starts_with('+') {
        if let Some(pos) = style.find('\n') {
            style.truncate(pos);
        }
        return Some(style);
    }
    while let Some(rest) = style.strip_prefix("posix-") {
        style = rest.to_string();
    }
    Some(style)
}

fn du_get_size_format(args_match: &ArgMatches) -> Result<DuSizeFormat, Box<dyn CTError>> {
    let last_option = [
        opt_flags::HUMAN_READABLE,
        opt_flags::SI,
        opt_flags::BYTES,
        opt_flags::BLOCK_SIZE_1K,
        opt_flags::BLOCK_SIZE_1M,
        opt_flags::BLOCK_SIZE,
    ]
    .into_iter()
    .filter(|option| {
        args_match.value_source(option) == Some(clap::parser::ValueSource::CommandLine)
    })
    .filter_map(|option| args_match.index_of(option).map(|index| (index, option)))
    .max_by_key(|(index, _)| *index)
    .map(|(_, option)| option);

    let size_format = match last_option {
        Some(opt_flags::HUMAN_READABLE) => DuSizeFormat::Human(1024),
        Some(opt_flags::SI) => DuSizeFormat::Human(1000),
        Some(opt_flags::BYTES) => DuSizeFormat::BlockSize(1),
        Some(opt_flags::BLOCK_SIZE_1K) => DuSizeFormat::BlockSize(1024),
        Some(opt_flags::BLOCK_SIZE_1M) => DuSizeFormat::BlockSize(1024 * 1024),
        Some(opt_flags::BLOCK_SIZE) => du_read_block_size_format(
            args_match
                .get_one::<String>(opt_flags::BLOCK_SIZE)
                .map(AsRef::as_ref),
        )?,
        Some(_) => unreachable!("all size options are handled"),
        None => du_read_block_size_format(None)?,
    };
    Ok(size_format)
}

fn du_get_time(args_match: &ArgMatches) -> Option<DuTime> {
    // 解析显示时间类型选项
    args_match.contains_id(opt_flags::TIME).then(|| {
        match args_match
            .get_one::<String>(opt_flags::TIME)
            .map(AsRef::as_ref)
        {
            None => DuTime::Modified,
            Some("ctime" | "status") => DuTime::Changed,
            Some("access" | "atime" | "use") => DuTime::Accessed,
            Some("birth" | "creation") => DuTime::Created,
            _ => unreachable!("should be caught by clap"),
        }
    })
}

fn du_parse_time_value(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("time value must not be empty".to_string());
    }

    [
        "atime", "access", "use", "ctime", "status", "birth", "creation",
    ]
    .into_iter()
    .find(|candidate| candidate.starts_with(value))
    .map(str::to_string)
    .ok_or_else(|| format!("invalid time value {value:?}"))
}

fn du_parse_time_style_value(value: &str) -> Result<String, String> {
    if value.starts_with('+') {
        return Ok(value.to_string());
    }
    if value.is_empty() {
        return Err("time style must not be empty".to_string());
    }

    ["full-iso", "long-iso", "iso"]
        .into_iter()
        .find(|candidate| candidate.starts_with(value))
        .map(str::to_string)
        .ok_or_else(|| format!("invalid time style {value:?}"))
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

fn du_check_option_conflicts(args_match: &ArgMatches) -> CTResult<()> {
    if args_match.get_flag(opt_flags::ALL) && args_match.get_flag(opt_flags::SUMMARIZE) {
        Err(DuError::SummarizeAllConflict.into())
    } else {
        Ok(())
    }
}

fn du_get_timestamp(du_time: DuTime, du_stat: &DuStat) -> Result<DuTimestamp, DuError> {
    match du_time {
        DuTime::Modified => Ok(du_stat.modified),
        DuTime::Accessed => Ok(du_stat.accessed),
        DuTime::Changed => Ok(du_stat.changed),
        DuTime::Created => du_stat.created.ok_or(DuError::InvalidTimeArg),
    }
}

fn du_parse_time_style(option: Option<&str>) -> CTResult<&str> {
    match option {
        Some(s) => match s {
            "full-iso" => Ok("%Y-%m-%d %H:%M:%S.%f %z"),
            "long-iso" => Ok("%Y-%m-%d %H:%M"),
            "iso" => Ok("%Y-%m-%d"),
            _ if s.starts_with('+') => Ok(s),
            _ => Err(DuError::InvalidTimeStyleArg(s.into()).into()),
        },
        None => Ok("%Y-%m-%d %H:%M"),
    }
}

fn du_parse_depth(du_max_depth_str: Option<&str>, summarize: bool) -> CTResult<Option<usize>> {
    match du_max_depth_str {
        Some(s) => {
            let depth = du_parse_depth_limit(s).ok_or_else(|| {
                Box::new(DuError::InvalidMaxDepthArg(s.into())) as Box<dyn CTError>
            })?;
            if summarize && depth != Some(0) {
                Err(DuError::SummarizeDepthConflict(s.into()).into())
            } else {
                Ok(depth)
            }
        }
        None => Ok(None),
    }
}

fn du_summarize_depth_warning(summarize: bool, max_depth: Option<usize>) -> Option<&'static str> {
    (summarize && max_depth == Some(0)).then_some("summarizing is the same as using --max-depth=0")
}

fn du_parse_depth_limit(depth: &str) -> Option<Option<usize>> {
    let value = du_parse_depth_number(depth)?;
    if value < i64::MIN as i128 || value > isize::MAX as i128 {
        return None;
    }

    if value < 0 {
        Some(None)
    } else {
        Some(Some(value as usize))
    }
}

fn du_parse_depth_number(depth: &str) -> Option<i128> {
    let depth = depth.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (negative, digits) = match depth.as_bytes().first() {
        Some(b'-') => (true, &depth[1..]),
        Some(b'+') => (false, &depth[1..]),
        Some(_) => (false, depth),
        None => return None,
    };

    if digits.is_empty() {
        return None;
    }

    let (radix, digits) = if let Some(rest) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        if rest.is_empty() {
            return None;
        }
        (16, rest)
    } else if digits.len() > 1 && digits.starts_with('0') {
        (8, digits)
    } else {
        (10, digits)
    };

    let mut value = 0i128;
    for byte in digits.bytes() {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        if digit as u32 >= radix {
            return None;
        }
        value = value
            .checked_mul(radix as i128)?
            .checked_add(digit as i128)?;
    }

    if negative {
        value.checked_neg()
    } else {
        Some(value)
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("du.about");
    let usage_description = t!("du.usage");
    let args = vec![
        Arg::new(opt_flags::HELP)
            .long(opt_flags::HELP)
            .help(t!("du.clap.help"))
            .action(ArgAction::Help),
        Arg::new(opt_flags::ALL)
            .short('a')
            .long(opt_flags::ALL)
            .help(t!("du.clap.all"))
            .overrides_with(opt_flags::ALL)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::APPARENT_SIZE)
            .long(opt_flags::APPARENT_SIZE)
            .help(
                "print apparent sizes, rather than disk usage \
                although the apparent size is usually smaller, it may be larger due to holes \
                in ('sparse') files, internal fragmentation, indirect blocks, and the like"
            )
            .overrides_with(opt_flags::APPARENT_SIZE)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::BLOCK_SIZE)
            .short('B')
            .long(opt_flags::BLOCK_SIZE)
            .value_name("SIZE")
            .overrides_with(opt_flags::BLOCK_SIZE)
            .help(
                "scale sizes by SIZE before printing them. \
                E.g., '-BM' prints sizes in units of 1,048,576 bytes. See SIZE format below."
            ),
        Arg::new(opt_flags::BYTES)
            .short('b')
            .long("bytes")
            .help(t!("du.clap.bytes"))
            .overrides_with(opt_flags::BYTES)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::TOTAL)
            .long("total")
            .short('c')
            .help(t!("du.clap.total"))
            .overrides_with(opt_flags::TOTAL)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::MAX_DEPTH)
            .short('d')
            .long("max-depth")
            .value_name("N")
            .overrides_with(opt_flags::MAX_DEPTH)
            .allow_hyphen_values(true)
            .help(
                "print the total for a directory (or file, with --all) \
                only if it is N or fewer levels below the command \
                line argument;  --max-depth=0 is the same as --summarize"
            ),
        Arg::new(opt_flags::HUMAN_READABLE)
            .long("human-readable")
            .short('h')
            .help(t!("du.clap.human_readable"))
            .overrides_with(opt_flags::HUMAN_READABLE)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::INODES)
            .long(opt_flags::INODES)
            .help(
                "list inode usage information instead of block usage like --block-size=1K"
            )
            .overrides_with(opt_flags::INODES)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::BLOCK_SIZE_1K)
            .short('k')
            .help(t!("du.clap.block_size_1k"))
            .overrides_with(opt_flags::BLOCK_SIZE_1K)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::COUNT_LINKS)
            .short('l')
            .long("count-links")
            .help(t!("du.clap.count_links"))
            .overrides_with(opt_flags::COUNT_LINKS)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::DEREFERENCE)
            .short('L')
            .long(opt_flags::DEREFERENCE)
            .help(t!("du.clap.dereference"))
            .overrides_with(opt_flags::DEREFERENCE)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::DEREFERENCE_ARGS)
            .short('D')
            .visible_short_alias('H')
            .long(opt_flags::DEREFERENCE_ARGS)
            .help(t!("du.clap.dereference_args"))
            .overrides_with(opt_flags::DEREFERENCE_ARGS)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::NO_DEREFERENCE)
            .short('P')
            .long(opt_flags::NO_DEREFERENCE)
            .help(t!("du.clap.no_dereference"))
            .overrides_with(opt_flags::DEREFERENCE)
            .overrides_with(opt_flags::NO_DEREFERENCE)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::BLOCK_SIZE_1M)
            .short('m')
            .help(t!("du.clap.block_size_1m"))
            .overrides_with(opt_flags::BLOCK_SIZE_1M)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::NULL)
            .short('0')
            .long("null")
            .help(t!("du.clap.null"))
            .overrides_with(opt_flags::NULL)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::SEPARATE_DIRS)
            .short('S')
            .long("separate-dirs")
            .help(t!("du.clap.separate_dirs"))
            .overrides_with(opt_flags::SEPARATE_DIRS)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::SUMMARIZE)
            .short('s')
            .long("summarize")
            .help(t!("du.clap.summarize"))
            .overrides_with(opt_flags::SUMMARIZE)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::SI)
            .long(opt_flags::SI)
            .help(t!("du.clap.si"))
            .overrides_with(opt_flags::SI)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::ONE_FILE_SYSTEM)
            .short('x')
            .long(opt_flags::ONE_FILE_SYSTEM)
            .help(t!("du.clap.one_file_system"))
            .overrides_with(opt_flags::ONE_FILE_SYSTEM)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::THRESHOLD)
            .short('t')
            .long(opt_flags::THRESHOLD)
            .value_name("SIZE")
            .num_args(1)
            .allow_hyphen_values(true)
            .overrides_with(opt_flags::THRESHOLD)
            .help(t!("du.clap.threshold")),
        Arg::new(opt_flags::VERBOSE)
            .short('v')
            .long("verbose")
            .help(t!("du.clap.verbose"))
            .overrides_with(opt_flags::VERBOSE)
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::EXCLUDE)
            .long(opt_flags::EXCLUDE)
            .value_name("PATTERN")
            .help(t!("du.clap.exclude"))
            .action(ArgAction::Append),
        Arg::new(opt_flags::EXCLUDE_FROM)
            .short('X')
            .long("exclude-from")
            .value_name("FILE")
            .value_hint(clap::ValueHint::FilePath)
            .help(t!("du.clap.exclude_from"))
            .action(ArgAction::Append),
        Arg::new(opt_flags::FILES0_FROM)
            .long("files0-from")
            .value_name("FILE")
            .value_hint(clap::ValueHint::FilePath)
            .help(t!("du.clap.files0_from"))
            .action(ArgAction::Append),
        Arg::new(opt_flags::TIME)
            .long(opt_flags::TIME)
            .value_name("WORD")
            .require_equals(true)
            .num_args(0..)
            .overrides_with(opt_flags::TIME)
            .value_parser(du_parse_time_value)
            .help(
                "show time of the last modification of any file in the \
                directory, or any of its subdirectories. If WORD is given, show time as WORD instead \
                of modification time: atime, access, use, ctime, status, birth or creation"
            ),
        Arg::new(opt_flags::TIME_STYLE)
            .long(opt_flags::TIME_STYLE)
            .value_name("STYLE")
            .overrides_with(opt_flags::TIME_STYLE)
            .value_parser(du_parse_time_style_value)
            .help(
                "show times using style STYLE: \
                full-iso, long-iso, iso, +FORMAT FORMAT is interpreted like 'date'"
            ),
        Arg::new(opt_flags::FILE)
            .hide(true)
            .value_hint(clap::ValueHint::AnyPath)
            .value_parser(clap::builder::PathBufValueParser::new())
            .action(ArgAction::Append),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .after_help(t!("du.after_help"))
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
        let value = str.trim_start_matches(|c: char| c.is_ascii_whitespace());
        // 根据字符串是否以'-'或'+'开头，计算出实际大小字符串的起始位置。
        let str_offset = usize::from(value.starts_with(&['-', '+'][..]));
        let magnitude = &value[str_offset..];
        if magnitude.ends_with('b') {
            return Err(ParseSizeError::InvalidSuffix(str.to_string()));
        }

        // 解析大小。
        let str_size = parse_size_u64(magnitude)?;
        let negative = value.starts_with('-');
        let max_magnitude = if negative {
            i64::MAX as u64 + 1
        } else {
            i64::MAX as u64
        };
        if str_size > max_magnitude {
            return Err(ParseSizeError::SizeTooBig(str.to_string()));
        }

        // 如果字符串以'-'开头，判断解析出的大小是否为0，为0则报错。
        if negative {
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
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_count_links_counts_repeated_directory_arguments() {
        let temp_dir = tempfile::tempdir().unwrap();
        let tree = temp_dir.path().join("tree");
        fs::create_dir(&tree).unwrap();
        fs::write(tree.join("file"), b"x").unwrap();

        let args = vec![
            OsString::from("du"),
            OsString::from("-l"),
            OsString::from("-c"),
            tree.as_os_str().to_os_string(),
            tree.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();
        let roots: Vec<_> = semantic
            .rows
            .iter()
            .filter(|row| row.kind == "entry" && row.depth == 0)
            .collect();
        let total = semantic
            .rows
            .iter()
            .find(|row| row.kind == "total")
            .unwrap();

        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].measured_size, roots[1].measured_size);
        assert_eq!(total.measured_size, roots[0].measured_size * 2);
    }

    #[cfg(unix)]
    #[test]
    fn test_top_level_access_errors_preserve_errno() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir.path().join("file");
        let child = file.join("child");
        let loop_link = temp_dir.path().join("loop");
        fs::write(&file, b"x").unwrap();
        symlink("loop", &loop_link).unwrap();

        let not_dir = du_native_semantic(
            [OsString::from("du"), child.as_os_str().to_os_string()].into_iter(),
        )
        .unwrap();
        let symlink_loop = du_native_semantic(
            [
                OsString::from("du"),
                OsString::from("-L"),
                loop_link.as_os_str().to_os_string(),
            ]
            .into_iter(),
        )
        .unwrap();

        assert_eq!(not_dir.exit_code, 1);
        assert!(not_dir.stderr_text.contains("Not a directory"));
        assert_eq!(symlink_loop.exit_code, 1);
        assert!(
            symlink_loop
                .stderr_text
                .contains("Too many levels of symbolic links")
        );
    }

    #[test]
    fn test_access_error_uses_gnu_shell_escape_quoting() {
        let error = io::Error::from(io::ErrorKind::NotFound);
        let message = du_access_error_message(Path::new("bad\n\x01name"), &error);

        assert!(message.starts_with("cannot access 'bad'$'\\n\\001''name': "));
    }

    #[test]
    fn test_access_error_uses_localized_template_and_detail() {
        assert_eq!(
            du_format_access_error("无法访问 %s", "'missing'", "没有那个文件或目录"),
            "无法访问 'missing': 没有那个文件或目录"
        );
    }

    #[test]
    fn test_separate_dirs_total_includes_subdirectories() {
        let temp_dir = tempfile::tempdir().unwrap();
        let tree = temp_dir.path().join("tree");
        let child = tree.join("child");
        fs::create_dir_all(&child).unwrap();
        fs::write(tree.join("root-file"), b"root").unwrap();
        fs::write(child.join("child-file"), b"child").unwrap();

        let args = vec![
            OsString::from("du"),
            OsString::from("-S"),
            OsString::from("-c"),
            OsString::from("--apparent-size"),
            OsString::from("-B1"),
            tree.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();
        let entry_total: u64 = semantic
            .rows
            .iter()
            .filter(|row| row.kind == "entry")
            .map(|row| row.measured_size)
            .sum();
        let total = semantic
            .rows
            .iter()
            .find(|row| row.kind == "total")
            .unwrap();

        assert_eq!(entry_total, 9);
        assert_eq!(total.measured_size, entry_total);
    }

    #[cfg(unix)]
    #[test]
    fn test_traversal_beyond_path_max_uses_directory_fds() {
        use nix::sys::stat::mkdirat;
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().join("deep");
        fs::create_dir(&root).unwrap();
        let component = OsStr::new("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd");
        let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC;
        let mut directory = Dir::open(&root, flags, Mode::empty()).unwrap();
        let mut display_path = root.clone();

        for _ in 0..75 {
            mkdirat(
                Some(directory.as_raw_fd()),
                component,
                Mode::from_bits_truncate(0o755),
            )
            .unwrap();
            directory =
                Dir::openat(Some(directory.as_raw_fd()), component, flags, Mode::empty()).unwrap();
            display_path.push(component);
        }
        assert!(display_path.as_os_str().as_bytes().len() > 4096);

        let args = vec![
            OsString::from("du"),
            OsString::from("-s"),
            root.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();

        assert_eq!(semantic.exit_code, 0);
        assert!(semantic.stderr_text.is_empty());
    }

    #[test]
    fn test_grand_total_includes_latest_time() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir.path().join("file");
        fs::write(&file, b"x").unwrap();
        let args = vec![
            OsString::from("du"),
            OsString::from("--time"),
            OsString::from("--time-style=iso"),
            OsString::from("-c"),
            file.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();
        let entry = semantic
            .rows
            .iter()
            .find(|row| row.kind == "entry")
            .unwrap();
        let total = semantic
            .rows
            .iter()
            .find(|row| row.kind == "total")
            .unwrap();
        let total_fields: Vec<_> = semantic
            .classic_text
            .lines()
            .last()
            .unwrap()
            .split('\t')
            .collect();

        assert_eq!(total.time_seconds, entry.time_seconds);
        assert_eq!(total.time_display, entry.time_display);
        assert_eq!(total_fields.len(), 3);
        assert_eq!(total_fields[1], entry.time_display.as_deref().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn test_full_iso_preserves_timestamp_nanoseconds() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir.path().join("file");
        fs::write(&file, b"x").unwrap();
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(UNIX_EPOCH + Duration::new(981_173_106, 123_456_789)),
            )
            .unwrap();

        let args = vec![
            OsString::from("du"),
            OsString::from("--time"),
            OsString::from("--time-style=full-iso"),
            file.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();
        let time_display = semantic
            .rows
            .iter()
            .find(|row| row.kind == "entry")
            .unwrap()
            .time_display
            .as_deref()
            .unwrap();

        assert!(time_display.contains(".123456789 "));
    }

    #[cfg(unix)]
    #[test]
    fn test_pre_epoch_time_is_formatted_without_panic() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir.path().join("file");
        fs::write(&file, b"x").unwrap();
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(UNIX_EPOCH - Duration::from_secs(315_532_800)),
            )
            .unwrap();

        let args = vec![
            OsString::from("du"),
            OsString::from("--time"),
            OsString::from("--time-style=+%Y"),
            file.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();
        let entry = semantic
            .rows
            .iter()
            .find(|row| row.kind == "entry")
            .unwrap();

        assert_eq!(entry.time_display.as_deref(), Some("1960"));
        assert!(entry.time_seconds.unwrap() < 0);
    }

    #[cfg(unix)]
    #[test]
    fn test_ctime_selection_uses_status_change_time() {
        use std::os::unix::fs::MetadataExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir.path().join("file");
        fs::write(&file, b"x").unwrap();
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(981_158_400)),
            )
            .unwrap();
        let metadata = fs::metadata(&file).unwrap();
        assert_ne!(metadata.ctime(), metadata.mtime());

        let args = vec![
            OsString::from("du"),
            OsString::from("--time=ctime"),
            OsString::from("--time-style=+%s"),
            file.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();
        let entry = semantic
            .rows
            .iter()
            .find(|row| row.kind == "entry")
            .unwrap();

        assert_eq!(entry.time_seconds, Some(metadata.ctime()));
    }

    #[test]
    fn test_du_long_option_with_unexpected_argument() {
        assert_eq!(
            du_long_option_with_unexpected_argument("--ino=value"),
            Some("inodes")
        );
        assert_eq!(du_long_option_with_unexpected_argument("--block=1K"), None);
        assert_eq!(du_long_option_with_unexpected_argument("--a=value"), None);
    }

    #[test]
    fn test_du_short_option_value_detection() {
        assert!(du_short_option_consumes_next("-hB"));
        assert!(du_short_option_consumes_next("-d"));
        assert!(!du_short_option_consumes_next("-B1K"));
        assert!(!du_short_option_consumes_next("-hh"));
    }

    #[test]
    fn test_du_short_option_cluster_reports_invalid_character() {
        assert_eq!(du_invalid_short_option_byte(OsStr::new("-hhq")), Some(b'q'));
        assert_eq!(du_invalid_short_option_byte(OsStr::new("-qh")), Some(b'q'));
        assert_eq!(du_invalid_short_option_byte(OsStr::new("-hBq")), None);
        assert_eq!(du_invalid_short_option_byte(OsStr::new("-hh")), None);
    }

    #[cfg(unix)]
    #[test]
    fn test_du_multibyte_short_option_uses_first_byte() {
        use std::os::unix::ffi::OsStringExt;

        let option = OsString::from_vec(vec![b'-', 0xc3, 0xa9]);
        let invalid = du_invalid_short_option_byte(option.as_os_str());
        let mut diagnostic = Vec::new();

        write_du_invalid_option_diagnostic(&mut diagnostic, "du", invalid.unwrap()).unwrap();

        assert_eq!(invalid, Some(0xc3));
        assert_eq!(
            diagnostic,
            b"du: invalid option -- '\xc3'\nTry 'du --help' for more information.\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_non_utf8_operand_reaches_traversal() {
        use std::os::unix::ffi::OsStringExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir
            .path()
            .join(OsString::from_vec(vec![b'r', b'a', b'w', 0xff]));
        fs::write(&file, b"x").unwrap();

        let args = vec![
            OsString::from("du"),
            OsString::from("--"),
            file.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();

        assert_eq!(semantic.exit_code, 0);
        assert_eq!(semantic.rows.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn test_path_output_preserves_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(vec![b'r', b'a', b'w', 0xff]));
        let mut output = Vec::new();

        du_write_path(&mut output, &path).unwrap();

        assert_eq!(output, b"raw\xff");
    }

    #[cfg(unix)]
    #[test]
    fn test_exclude_check_does_not_panic_on_non_utf8_name() {
        use std::os::unix::ffi::OsStringExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join(OsString::from_vec(vec![b'r', b'a', b'w', 0xff])),
            b"x",
        )
        .unwrap();

        let args = vec![
            OsString::from("du"),
            OsString::from("-a"),
            OsString::from("--exclude=nomatch"),
            root.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();

        assert_eq!(semantic.exit_code, 0);
        assert_eq!(semantic.rows.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn test_exclude_file_matches_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().join("root");
        let excludes = temp_dir.path().join("excludes");
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join(OsString::from_vec(vec![b'r', b'a', b'w', 0xff])),
            b"x",
        )
        .unwrap();
        fs::write(&excludes, b"raw\xff\n").unwrap();

        let args = vec![
            OsString::from("du"),
            OsString::from("-a"),
            OsString::from("-X"),
            excludes.as_os_str().to_os_string(),
            root.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();

        assert_eq!(semantic.exit_code, 0);
        assert_eq!(semantic.rows.len(), 1);
    }

    #[test]
    fn test_du_repeated_boolean_flag_is_accepted() {
        let matches = ct_app().try_get_matches_from(["du", "-hh"]).unwrap();

        assert!(matches.get_flag(opt_flags::HUMAN_READABLE));
    }

    #[test]
    fn test_incomplete_exclude_bracket_is_literal() {
        let bracket = DuExcludePattern::new(b"[".to_vec()).unwrap();
        let mixed = DuExcludePattern::new(b"foo[bar*".to_vec()).unwrap();

        assert!(du_exclude_pattern_matches_path(&bracket, Path::new("[")));
        assert!(du_exclude_pattern_matches_path(
            &mixed,
            Path::new("foo[bar-suffix")
        ));
        assert!(!du_exclude_pattern_matches_path(
            &mixed,
            Path::new("foobar-suffix")
        ));
    }

    #[test]
    fn test_exclude_pattern_matches_path_suffix() {
        let pattern = DuExcludePattern::new(b"sub/leaf".to_vec()).unwrap();

        assert!(du_exclude_pattern_matches_path(
            &pattern,
            Path::new("root/sub/leaf")
        ));
        assert!(!du_exclude_pattern_matches_path(
            &pattern,
            Path::new("root/other/leaf")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_excluded_dangling_link_is_skipped_before_stat() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().join("root");
        fs::create_dir(&root).unwrap();
        symlink("missing", root.join("dangling")).unwrap();

        let args = vec![
            OsString::from("du"),
            OsString::from("-L"),
            OsString::from("--exclude=dangling"),
            root.as_os_str().to_os_string(),
        ];
        let semantic = du_native_semantic(args.into_iter()).unwrap();

        assert_eq!(semantic.exit_code, 0);
        assert!(semantic.stderr_text.is_empty());
        assert!(!semantic.classic_text.contains("dangling"));
    }

    #[test]
    fn test_du_repeated_block_size_uses_last_value() {
        let matches = ct_app()
            .try_get_matches_from(["du", "-B1", "-B1K"])
            .unwrap();

        assert_eq!(
            matches
                .get_one::<String>(opt_flags::BLOCK_SIZE)
                .map(String::as_str),
            Some("1K")
        );
    }

    #[test]
    fn test_du_block_size_accepts_leading_plus_and_ascii_whitespace() {
        assert_eq!(du_parse_block_size_value("+1"), Ok(1));
        assert_eq!(du_parse_block_size_value(" \t1K"), Ok(1024));
    }

    #[test]
    fn test_du_block_size_rejects_lowercase_b_suffix() {
        assert!(matches!(
            du_parse_block_size_value("1b"),
            Err(ParseSizeError::InvalidSuffix(_))
        ));
    }

    #[test]
    fn test_du_human_readable_uses_gnu_ceiling_rounding() {
        assert_eq!(du_human_readable(1025, 1024), "1.1K");
        assert_eq!(du_human_readable(10239, 1024), "10K");
        assert_eq!(du_human_readable(1048575, 1024), "1.0M");
        assert_eq!(du_human_readable(9999, 1000), "10k");
    }

    #[test]
    fn test_du_human_readable_localizes_decimal_point() {
        assert_eq!(
            du_localize_decimal_point_with("1.0K".to_string(), ","),
            "1,0K"
        );
        assert_eq!(
            du_localize_decimal_point_with("1.1K".to_string(), "٫"),
            "1٫1K"
        );
    }

    #[test]
    fn test_du_repeated_max_depth_uses_last_value() {
        let matches = ct_app().try_get_matches_from(["du", "-d0", "-d1"]).unwrap();

        assert_eq!(
            matches
                .get_one::<String>(opt_flags::MAX_DEPTH)
                .map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn test_du_max_depth_accepts_separated_negative_argument() {
        let matches = ct_app().try_get_matches_from(["du", "-d", "-1"]).unwrap();

        assert_eq!(du_get_max_depth(&matches, false).unwrap(), None);
    }

    #[test]
    fn test_du_summarize_accepts_zero_depth_with_warning() {
        let matches = ct_app().try_get_matches_from(["du", "-s", "-d0"]).unwrap();
        let max_depth = du_get_max_depth(&matches, true).unwrap();

        assert_eq!(max_depth, Some(0));
        assert_eq!(
            du_summarize_depth_warning(true, max_depth),
            Some("summarizing is the same as using --max-depth=0")
        );
    }

    #[test]
    fn test_du_repeated_threshold_uses_last_value() {
        let matches = ct_app().try_get_matches_from(["du", "-t1", "-t2"]).unwrap();

        assert_eq!(
            matches
                .get_one::<String>(opt_flags::THRESHOLD)
                .map(String::as_str),
            Some("2")
        );
    }

    #[test]
    fn test_du_threshold_accepts_leading_ascii_whitespace() {
        assert!(matches!(
            DuThreshold::from_str(" \t1").unwrap(),
            DuThreshold::Lower(1)
        ));
        assert!(matches!(
            DuThreshold::from_str(" \t-1").unwrap(),
            DuThreshold::Upper(1)
        ));
    }

    #[test]
    fn test_du_threshold_enforces_signed_range() {
        assert!(matches!(
            DuThreshold::from_str("9223372036854775807").unwrap(),
            DuThreshold::Lower(9223372036854775807)
        ));
        assert!(matches!(
            DuThreshold::from_str("-9223372036854775808").unwrap(),
            DuThreshold::Upper(9223372036854775808)
        ));
        assert!(matches!(
            DuThreshold::from_str("9223372036854775808"),
            Err(ParseSizeError::SizeTooBig(_))
        ));
        assert!(matches!(
            DuThreshold::from_str("-9223372036854775809"),
            Err(ParseSizeError::SizeTooBig(_))
        ));
    }

    #[test]
    fn test_du_threshold_rejects_lowercase_b_suffix() {
        assert!(matches!(
            DuThreshold::from_str("1b"),
            Err(ParseSizeError::InvalidSuffix(_))
        ));
        assert!(matches!(
            DuThreshold::from_str("-1b"),
            Err(ParseSizeError::InvalidSuffix(_))
        ));
    }

    #[test]
    fn test_du_repeated_time_uses_last_value() {
        let matches = ct_app()
            .try_get_matches_from(["du", "--time=atime", "--time=ctime"])
            .unwrap();

        assert_eq!(
            matches
                .get_one::<String>(opt_flags::TIME)
                .map(String::as_str),
            Some("ctime")
        );
    }

    #[test]
    fn test_du_time_value_accepts_abbreviation() {
        let matches = ct_app().try_get_matches_from(["du", "--time=at"]).unwrap();

        assert_eq!(
            matches
                .get_one::<String>(opt_flags::TIME)
                .map(String::as_str),
            Some("atime")
        );
    }

    #[test]
    fn test_du_time_style_accepts_abbreviation() {
        let matches = ct_app()
            .try_get_matches_from(["du", "--time-style=full"])
            .unwrap();

        assert_eq!(
            matches
                .get_one::<String>(opt_flags::TIME_STYLE)
                .map(String::as_str),
            Some("full-iso")
        );
    }

    #[test]
    fn test_du_repeated_time_style_uses_last_value() {
        let matches = ct_app()
            .try_get_matches_from(["du", "--time", "--time-style=iso", "--time-style=long-iso"])
            .unwrap();

        assert_eq!(
            matches
                .get_one::<String>(opt_flags::TIME_STYLE)
                .map(String::as_str),
            Some("long-iso")
        );
    }

    #[test]
    fn test_du_repeated_files0_from_uses_last_value() {
        let matches = ct_app()
            .try_get_matches_from(["du", "--files0-from=list-a", "--files0-from=list-b"])
            .unwrap();

        assert_eq!(
            du_last_files0_from(&matches).map(String::as_str),
            Some("list-b")
        );
    }

    #[test]
    fn test_du_posix_option_order_stops_at_first_operand() {
        let mut args = ["du", "-B", "1K", "tree", "-a"]
            .into_iter()
            .map(OsString::from)
            .collect();

        du_apply_posix_option_order(&mut args, true);

        assert_eq!(
            args,
            ["du", "-B", "1K", "--", "tree", "-a"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_du_output_size_uses_last_format_option() {
        let cases: &[(&[&str], DuSizeFormat)] = &[
            (&["du", "-h", "-k"], DuSizeFormat::BlockSize(1024)),
            (&["du", "-k", "-m"], DuSizeFormat::BlockSize(1024 * 1024)),
            (&["du", "-m", "-B2K"], DuSizeFormat::BlockSize(2048)),
            (&["du", "-B2K", "-b"], DuSizeFormat::BlockSize(1)),
            (&["du", "-b", "--si"], DuSizeFormat::Human(1000)),
            (&["du", "--si", "-h"], DuSizeFormat::Human(1024)),
        ];

        for (args, expected) in cases {
            let matches = ct_app().try_get_matches_from(args.iter().copied()).unwrap();

            assert_eq!(&du_get_size_format(&matches).unwrap(), expected);
        }

        let matches = ct_app().try_get_matches_from(["du", "-b", "-k"]).unwrap();
        assert!(matches.get_flag(opt_flags::BYTES));
        assert_eq!(
            du_get_size_format(&matches).unwrap(),
            DuSizeFormat::BlockSize(1024)
        );
    }

    #[test]
    fn test_du_dereference_uses_last_option() {
        let files_path = vec![PathBuf::from("tree/link")];
        let cases: &[(&[&str], DuDeref)] = &[
            (&["du", "-H", "-P"], DuDeref::None),
            (&["du", "-P", "-H"], DuDeref::Args(files_path.clone())),
            (&["du", "-H", "-L"], DuDeref::All),
            (&["du", "-L", "-H"], DuDeref::Args(files_path.clone())),
            (&["du", "-L", "-P"], DuDeref::None),
            (&["du", "-P", "-L"], DuDeref::All),
        ];

        for (args, expected) in cases {
            let matches = ct_app().try_get_matches_from(args.iter().copied()).unwrap();

            assert!(du_get_dereference(&matches, &files_path) == expected.clone());
        }
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Du;

        // 测试 name 方法
        assert_eq!(tool.name(), "du");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("du"));

        // 测试 execute 方法
        let args = vec![OsString::from("du")];
        assert!(tool.execute(&args).is_ok());
    }

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

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_flag(HUMAN_READABLE));
            assert!(matches.get_flag(INODES));
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

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(COUNT_LINKS));
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

        #[test]
        fn test_ct_app_no_dereference() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-P"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(NO_DEREFERENCE).unwrap());
        }

        #[test]
        fn test_ct_app_no_dereference_whole() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--no-dereference"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(NO_DEREFERENCE).unwrap());
        }

        #[test]
        fn test_ct_app_m() {
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
        fn test_ct_app_null() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--null"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(NULL).unwrap());
        }

        #[test]
        fn test_ct_app_zero() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-0"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(NULL).unwrap());
        }

        #[test]
        fn test_ct_app_separate_dirs() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-S"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(SEPARATE_DIRS).unwrap());
        }

        #[test]
        fn test_ct_app_separate_dirs_whole() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--separate-dirs"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(SEPARATE_DIRS).unwrap());
        }

        #[test]
        fn test_ct_app_summarize() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-s"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(SUMMARIZE).unwrap());
        }

        #[test]
        fn test_ct_app_summarize_whole() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--summarize"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(SUMMARIZE).unwrap());
        }

        #[test]
        fn test_ct_app_si() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--si"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(SI).unwrap());
        }
        #[test]
        fn test_ct_app_summarize_si() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-s", "--si"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(SUMMARIZE).unwrap());
            assert!(matches.get_one::<bool>(SI).unwrap());
        }

        #[test]
        fn test_ct_app_summarize_si_whole() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--summarize", "--si"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(SUMMARIZE).unwrap());
            assert!(matches.get_one::<bool>(SI).unwrap());
        }

        #[test]
        fn test_ct_app_s_summarize_si_whole() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-s", "--summarize", "--si"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_flag(SUMMARIZE));
            assert!(matches.get_flag(SI));
        }

        #[test]
        fn test_ct_app_one_file_system() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-x"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(ONE_FILE_SYSTEM).unwrap());
        }

        #[test]
        fn test_ct_app_one_file_system_whole() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--one-file-system"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(ONE_FILE_SYSTEM).unwrap());
        }

        #[test]
        fn test_ct_app_t_threshold() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-t", "--threshold", "1KB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "--threshold");
            // assert_eq!(result.unwrap_err().kind(), ErrorKind::ArgumentConflict);
        }

        #[test]
        fn test_ct_app_threshold_1k() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1KB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1KB");
        }

        #[test]
        fn test_ct_app_threshold_1m() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1MB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1MB");
        }

        #[test]
        fn test_ct_app_threshold_1g() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1GB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1GB");
        }

        #[test]
        fn test_ct_app_threshold_1t() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1TB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1TB");
        }

        #[test]
        fn test_ct_app_threshold_1p() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1PB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1PB");
        }

        #[test]
        fn test_ct_app_threshold_1e() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1EB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1EB");
        }

        #[test]
        fn test_ct_app_threshold_1y() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1YB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1YB");
        }

        #[test]
        fn test_ct_app_threshold_1z() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1ZB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1ZB");
        }

        #[test]
        fn test_ct_app_t_1k() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-t", "1KB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1KB");
        }

        #[test]
        fn test_ct_app_t_1m() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-t", "1MB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1MB");
        }

        #[test]
        fn test_ct_app_t_1g() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-t", "1GB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1GB");
        }

        #[test]
        fn test_ct_app_t_1t() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-t", "1TB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1TB");
        }

        #[test]
        fn test_ct_app_t_1p() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-t", "1PB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1PB");
        }

        #[test]
        fn test_ct_app_t_1e() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-t", "1EB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1EB");
        }

        #[test]
        fn test_ct_app_t_1y() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-t", "1YB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1YB");
        }

        #[test]
        fn test_ct_app_t_1z() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-t", "1ZB"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<String>(THRESHOLD).unwrap(), "1ZB");
        }

        #[test]
        fn test_ct_app_verbose() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-v"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(VERBOSE).unwrap());
        }

        #[test]
        fn test_ct_app_verbose_whole() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--verbose"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(VERBOSE).unwrap());
        }

        #[test]
        fn test_ct_app_exclude() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--exclude", "*.txt"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
        }

        #[test]
        fn test_ct_app_verbose_exclude() {
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
                "--verbose",
                "--exclude",
                "*.txt",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_one::<bool>(VERBOSE).unwrap());
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
        }

        #[test]
        fn test_ct_app_x_from() {
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
            let args = vec![ctcore::ct_util_name(), dir, "-X", "*.txt"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
        }

        #[test]
        fn test_ct_app_exclude_from() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--exclude-from", "*.txt"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
        }

        #[test]
        fn test_ct_app_files0_from() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--files0-from", "*.txt"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(FILES0_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
        }

        #[test]
        fn test_ct_app_exclude_from_files0_from() {
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
                "--exclude-from",
                "*.txt",
                "--files0-from",
                "*.txt",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );

            assert_eq!(
                matches
                    .get_many::<String>(FILES0_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
        }

        #[test]
        fn test_ct_app_time_ctime() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--time=ctime"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Modified);
        }

        #[test]
        fn test_ct_app_time_status() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--time=status"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Modified);
        }

        #[test]
        fn test_ct_app_time_access() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--time=access"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Accessed);
        }

        #[test]
        fn test_ct_app_time_atime() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--time=atime"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Accessed);
        }

        #[test]
        fn test_ct_app_time_use() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--time=use"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Accessed);
        }

        #[test]
        fn test_ct_app_time_birth() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--time=birth"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Created);
        }

        #[test]
        fn test_ct_app_time_creation() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--time=creation"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Created);
        }

        #[test]
        fn test_ct_app_exclude_from_time_ctime() {
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
                "--exclude-from",
                "*.txt",
                "--time=ctime",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );

            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Modified);
        }

        #[test]
        fn test_ct_app_exclude_from_time_status() {
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
                "--exclude-from",
                "*.txt",
                "--time=status",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );

            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Modified);
        }

        #[test]
        fn test_ct_app_exclude_from_time_access() {
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
                "--exclude-from",
                "*.txt",
                "--time=access",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Accessed);
        }

        #[test]
        fn test_ct_app_exclude_from_time_atime() {
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
                "--exclude-from",
                "*.txt",
                "--time=atime",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Accessed);
        }

        #[test]
        fn test_ct_app_exclude_from_time_use() {
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
                "--exclude-from",
                "*.txt",
                "--time=use",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Accessed);
        }

        #[test]
        fn test_ct_app_exclude_from_time_birth() {
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
                "--exclude-from",
                "*.txt",
                "--time=birth",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Created);
        }

        #[test]
        fn test_ct_app_exclude_from_time_creation() {
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
                "--exclude-from",
                "*.txt",
                "--time=creation",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Created);
        }

        #[test]
        fn test_ct_app_time_style_full_iso() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--time-style", "full-iso"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(TIME_STYLE)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["full-iso"]
            );
        }

        #[test]
        fn test_ct_app_time_style_long_iso() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--time-style", "long-iso"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(TIME_STYLE)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["long-iso"]
            );
        }

        #[test]
        fn test_ct_app_time_style_iso() {
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
            let args = vec![ctcore::ct_util_name(), dir, "--time-style", "iso"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(TIME_STYLE)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["iso"]
            );
        }

        #[test]
        fn test_ct_app_exclude_from_time_style_full_iso() {
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
                "--exclude-from",
                "*.txt",
                "--time-style",
                "full-iso",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );

            assert_eq!(
                matches
                    .get_many::<String>(TIME_STYLE)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["full-iso"]
            );
        }

        #[test]
        fn test_ct_app_exclude_from_time_style_long_iso() {
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
                "--exclude-from",
                "*.txt",
                "--time-style",
                "long-iso",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
            assert_eq!(
                matches
                    .get_many::<String>(TIME_STYLE)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["long-iso"]
            );
        }

        #[test]
        fn test_ct_app_exclude_from_time_style_iso() {
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
                "--exclude-from",
                "*.txt",
                "--time-style",
                "iso",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );
            assert_eq!(
                matches
                    .get_many::<String>(TIME_STYLE)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["iso"]
            );
        }

        #[test]
        fn test_ct_app_exclude_from_files0_from_time_time_style_iso() {
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
                "--exclude-from",
                "*.txt",
                "--files0-from",
                "*.txt",
                "--time=ctime",
                "--time-style",
                "iso",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches
                    .get_many::<String>(EXCLUDE_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );

            assert_eq!(
                matches
                    .get_many::<String>(FILES0_FROM)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["*.txt"]
            );

            assert_eq!(
                matches
                    .get_many::<String>(TIME_STYLE)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["iso"]
            );

            let result = match matches.get_one::<String>(TIME).map(AsRef::as_ref) {
                None | Some("ctime" | "status") => DuTime::Modified,
                Some("access" | "atime" | "use") => DuTime::Accessed,
                Some("birth" | "creation") => DuTime::Created,
                _ => unreachable!("should be caught by clap"),
            };

            assert_eq!(result, DuTime::Modified);
        }
    }
    mod test_ct_main {
        use super::*;
        use std::ffi::OsString;

        use std::io::Write;
        use tempfile::Builder;

        #[test]
        fn test_ct_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = du_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_a() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir];

            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_all() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--all"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--apparent-size"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_all_apparent_size() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--all", "--apparent-size"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_block_size_1k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--block-size", "1K"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_block_size_1m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--block-size", "1M"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_block_size_1g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--block-size", "1g"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_block_size_1t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--block-size", "1T"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_block_size_1p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--block-size", "1P"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_block_size_1e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--block-size", "1E"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_block_size_1z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--block-size", "1Z"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_block_size_1y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--block-size", "1Y"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_apparent_size_block_size_1k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1K",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_block_size_1m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1M",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_block_size_1g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1g",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_block_size_1t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1T",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_block_size_1p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1P",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_block_size_1e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1E",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_block_size_1z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1Z",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_apparent_size_block_size_1y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--apparent-size",
                "--block-size",
                "1Y",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_bk() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-BK"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bm() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-BM"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bg() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-BG"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bt() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-BT"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bp() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-BP"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_be() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-BE"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bz() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-BZ"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_by() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-BY"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_apparent_size_bk() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--apparent-size", "-BK"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_bm() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--apparent-size", "-BM"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_bg() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--apparent-size", "-BG"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_bt() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--apparent-size", "-BT"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_bp() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--apparent-size", "-BP"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_be() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--apparent-size", "-BE"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_apparent_size_bz() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--apparent-size", "-BZ"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err()); //du: --block-size argument 'Z' too large
        }

        #[test]
        fn test_ct_main_apparent_size_by() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--apparent-size", "-BY"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err()); //du: --block-size argument 'Y' too large
        }

        #[test]
        fn test_ct_main_b() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-b"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_bytes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--bytes"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-c"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_total_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file).unwrap();
            let _ = test_file.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--total"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_block_size_flag_k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-k"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_block_size_flag_1m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-m"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_max_depth_zeros() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--max-depth", "0"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_max_depth_1() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--max-depth", "1"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_max_depth_2() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--max-depth", "2"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_max_depth_3() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--max-depth", "3"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_max_depth_4() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--max-depth", "4"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_max_depth_5() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--max-depth", "5"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d_zeros() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-d", "0"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d_1() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-d", "1"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d_2() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-d", "2"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d_3() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-d", "3"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d_4() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-d", "4"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_d_5() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-d", "5"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_human_readable() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-h"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_human_readable_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--human-readable"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_inodes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--inodes"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_human_readable_inodes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--human-readable", "--inodes"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_h_human_readable_inodes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "-h",
                "--human-readable",
                "--inodes",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-k"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_count_links() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-l"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_count_links_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--count-links"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_l_count_links_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-l", "--count-links"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_dereference() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-L"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_dereference_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--dereference"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_dereference_args() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-D"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_dereference_args_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--dereference-args"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_no_dereference() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-P"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_no_dereference_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--no-dereference"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-m"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--null"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-0"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_separate_dirs() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-S"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_separate_dirs_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--separate-dirs"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_summarize() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-s"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_summarize_all_conflict() {
            let args = [ctcore::ct_util_name(), "-s", "-a", "case"];
            let err = du_main(args.iter().map(OsString::from))
                .expect_err("summarize and all should conflict");

            assert_eq!(err.code(), 1);
            assert!(err.usage());
            assert_eq!(
                err.to_string(),
                "cannot both summarize and show all entries"
            );
        }

        #[test]
        fn test_ct_main_summarize_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--summarize"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_si() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--si"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_summarize_si() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-s", "--si"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_summarize_si_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--summarize", "--si"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_s_summarize_si_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-s", "--summarize", "--si"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_one_file_system() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-x"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_one_file_system_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--one-file-system"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_t_threshold() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-t", "--threshold", "1KB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_threshold_1k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--threshold", "1KB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_threshold_1m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--threshold", "1MB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_threshold_1g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--threshold", "1GB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_threshold_1t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--threshold", "1TB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_threshold_1p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--threshold", "1PB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_threshold_1e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--threshold", "1EB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_threshold_1y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--threshold", "1YB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_threshold_1z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--threshold", "1ZB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_t_1k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-t", "1KB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_t_1m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-t", "1MB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_t_1g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-t", "1GB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_t_1t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-t", "1TB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_t_1p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-t", "1PB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_t_1e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-t", "1EB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_t_1y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-t", "1YB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_t_1z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-t", "1ZB"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_verbose() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-v"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_verbose_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--verbose"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--exclude", filename];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_verbose_exclude() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--verbose",
                "--exclude",
                filename,
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_x_from() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "-X", filename];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--exclude-from", filename];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from_missing_file_returns_error() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let missing = temp_dir.path().join("missing-excludes");
            let missing = missing.to_str().unwrap();

            let args = [ctcore::ct_util_name(), dir, "-X", missing];
            let err = du_main(args.iter().map(OsString::from)).expect_err("missing -X file");

            assert_eq!(err.code(), 1);
        }

        #[test]
        fn test_ct_main_files0_from() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let files0 = sub_dir_path.join("files0.lst");
            let mut files0_file = File::create(&files0).unwrap();
            files0_file
                .write_all(format!("{dir}\0").as_bytes())
                .unwrap();
            let files0_name = files0.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--files0-from", files0_name];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from_files0_from() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let files0 = sub_dir_path.join("files0.lst");
            let mut files0_file = File::create(&files0).unwrap();
            files0_file
                .write_all(format!("{dir}\0").as_bytes())
                .unwrap();
            let files0_name = files0.to_str().unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--exclude-from",
                filename,
                "--files0-from",
                files0_name,
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_time_ctime() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--time=ctime"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_time_status() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--time=status"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_time_access() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--time=access"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_time_atime() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--time=atime"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_time_use() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--time=use"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_time_birth() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();
            let metadata = sub_dir_path
                .metadata()
                .expect("Failed to get file metadata");

            let args = [ctcore::ct_util_name(), dir, "--time=birth"];
            let result = du_main(args.iter().map(OsString::from));

            if metadata.created().is_ok() {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
        }

        #[test]
        fn test_ct_main_time_creation() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();
            let metadata = sub_dir_path
                .metadata()
                .expect("Failed to get file metadata");
            let args = [ctcore::ct_util_name(), dir, "--time=creation"];
            let result = du_main(args.iter().map(OsString::from));

            if metadata.created().is_ok() {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
        }

        #[test]
        fn test_ct_main_exclude_from_time_ctime() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--exclude-from",
                filename,
                "--time=ctime",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from_time_status() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--exclude-from",
                filename,
                "--time=status",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from_time_access() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--exclude-from",
                filename,
                "--time=access",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from_time_atime() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--exclude-from",
                filename,
                "--time=atime",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from_time_use() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--exclude-from",
                filename,
                "--time=use",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from_time_birth() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();
            let metadata = sub_dir_path
                .metadata()
                .expect("Failed to get file metadata");

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--exclude-from",
                filename,
                "--time=birth",
            ];
            let result = du_main(args.iter().map(OsString::from));

            if metadata.created().is_ok() {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
        }

        #[test]
        fn test_ct_main_exclude_from_time_creation() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();
            let metadata = sub_dir_path
                .metadata()
                .expect("Failed to get file metadata");

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--exclude-from",
                filename,
                "--time=creation",
            ];
            let result = du_main(args.iter().map(OsString::from));

            if metadata.created().is_ok() {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
        }

        #[test]
        fn test_ct_main_time_style_full_iso() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--time-style", "full-iso"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_time_style_long_iso() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--time-style", "long-iso"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_time_style_iso() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), dir, "--time-style", "iso"];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from_time_style_full_iso() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let file_name = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--exclude-from",
                file_name,
                "--time-style",
                "full-iso",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from_time_style_iso() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let file_name = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                dir,
                "--exclude-from",
                file_name,
                "--time-style",
                "iso",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_exclude_from_files0_from_time_time_style_iso() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_dir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let file_name = test_file_1.to_str().unwrap();
            let dir = temp_dir.path().to_str().unwrap();
            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let files0 = sub_dir_path.join("files0.lst");
            let mut files0_file = File::create(&files0).unwrap();
            files0_file
                .write_all(format!("{dir}\0").as_bytes())
                .unwrap();
            let files0_name = files0.to_str().unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--exclude-from",
                file_name,
                "--files0-from",
                files0_name,
                "--time=ctime",
                "--time-style",
                "iso",
            ];
            let result = du_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }
}
#[cfg(test)]
mod tests_fn {
    use super::*;
    use std::ffi::OsString;

    use crate::DuError::SummarizeDepthConflict;
    use crate::DuSizeFormat::BlockSize;
    use std::io::Write;
    use tempfile::Builder;

    use std::fs::File;

    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn test_read_block_size() {
        let test_data = [Some("1024".to_string()), Some("K".to_string()), None];
        for it in &test_data {
            assert!(matches!(du_read_block_size(it.as_deref()), Ok(1024)));
        }
    }
    #[test]
    fn test_read_block_size_valid_input_1024_bytes() {
        let input_str = "1024";
        let expected_output = 1024;
        let result = du_read_block_size(Some(input_str)).unwrap();
        assert_eq!(result, expected_output);
    }

    #[test]
    fn test_read_block_size_valid_input_512k() {
        let input_str = "512K";
        let expected_output = 524288;
        let result = du_read_block_size(Some(input_str)).unwrap();
        assert_eq!(result, expected_output);
    }

    #[test]
    fn test_read_block_size_valid_input_1m() {
        let input_str = "1M";
        let expected_output = 1048576;
        let result = du_read_block_size(Some(input_str)).unwrap();
        assert_eq!(result, expected_output);
    }

    #[test]
    fn test_read_block_size_valid_input_2g() {
        let input_str = "2G";
        let expected_output = 2147483648;
        let result = du_read_block_size(Some(input_str)).unwrap();
        assert_eq!(result, expected_output);
    }

    #[test]
    fn test_read_block_size_valid_input_1t() {
        let input_str = "1T";
        let expected_output = 1099511627776;
        let result = du_read_block_size(Some(input_str)).unwrap();
        assert_eq!(result, expected_output);
    }

    #[test]
    fn test_read_block_size_invalid_input_invalid_string() {
        let input_str = "invalid";
        let result = du_read_block_size(Some(input_str));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_block_size_invalid_input_empty_string() {
        let input_str = "";
        let result = du_read_block_size(Some(input_str));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_block_size_invalid_input_invalid_unit() {
        let input_str = "1024X";
        let result = du_read_block_size(Some(input_str));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_block_size_invalid_input_mixed_units() {
        let input_str = "kK";
        let result = du_read_block_size(Some(input_str));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_block_size_invalid_input_negative_value() {
        let input_str = "-1024";
        let result = du_read_block_size(Some(input_str));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_block_size_invalid_input_fractional_value() {
        let input_str = "1024.5";
        let result = du_read_block_size(Some(input_str));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_block_size_invalid_input_multiple_units() {
        let input_str = "1T1G";
        let result = du_read_block_size(Some(input_str));
        assert!(result.is_err());
    }

    fn create_temp_file_with_content(content: &str) -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test_file.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        temp_dir
    }

    fn create_temp_symlink(target: &Path, symlink_path: &Path) {
        symlink(target, symlink_path).unwrap();
    }

    #[test]
    fn test_stat_new_regular_file() {
        let temp_dir = create_temp_file_with_content("Hello, world!");
        let test_file_path = temp_dir.path().join("test_file.txt");
        let exclude_pattern = DuExcludePattern::new(b"test_file.txt".to_vec()).unwrap();
        let exclude_pattern = vec![exclude_pattern];
        let options = DuTraversalOptions {
            all: true,
            separate_dirs: false,
            one_file_system: true,
            dereference: DuDeref::All,
            count_links: false,
            verbose: false,
            excludes: exclude_pattern,
        };

        let stat_result = DuStat::new(&test_file_path, &options, true);

        assert!(stat_result.is_ok());

        let stat = stat_result.unwrap();
        assert_eq!(stat.path, test_file_path);
        assert!(!stat.is_dir);
        assert_eq!(stat.size, "Hello, world!".len() as u64);
    }

    #[test]
    fn test_stat_new_directory() {
        let temp_dir = TempDir::new().unwrap();
        let exclude_pattern = DuExcludePattern::new(b"test_file.txt".to_vec()).unwrap();
        let exclude_pattern = vec![exclude_pattern];
        let options = DuTraversalOptions {
            all: true,
            separate_dirs: false,
            one_file_system: true,
            dereference: DuDeref::All,
            count_links: false,
            verbose: false,
            excludes: exclude_pattern,
        };

        let stat_result = DuStat::new(temp_dir.path(), &options, true);

        assert!(stat_result.is_ok());

        let stat = stat_result.unwrap();
        assert_eq!(stat.path, temp_dir.path());
        assert!(stat.is_dir);
        assert_eq!(stat.size, 0);
    }

    #[test]
    fn test_stat_new_following_symlink() {
        let temp_dir = create_temp_file_with_content("Symlink target content");
        let test_file_path = temp_dir.path().join("test_file.txt");
        let symlink_temp_dir = TempDir::new().unwrap();
        let symlink_path = symlink_temp_dir.path().join("symlink.txt");

        create_temp_symlink(&test_file_path, &symlink_path);

        let exclude_pattern = DuExcludePattern::new(b"test_file.txt".to_vec()).unwrap();
        let exclude_pattern = vec![exclude_pattern];
        let options = DuTraversalOptions {
            all: true,
            separate_dirs: false,
            one_file_system: true,
            dereference: DuDeref::All,
            count_links: false,
            verbose: false,
            excludes: exclude_pattern,
        };

        let stat_result = DuStat::new(&symlink_path, &options, true);

        assert!(stat_result.is_ok());

        let stat = stat_result.unwrap();
        assert_eq!(stat.path, symlink_path);
        assert!(!stat.is_dir);
        assert_eq!(stat.size, "Symlink target content".len() as u64);
    }

    #[test]
    fn test_stat_new_keeps_directory_symlink_apparent_size() {
        let temp_dir = TempDir::new().unwrap();
        fs::create_dir(temp_dir.path().join("sub")).unwrap();
        let symlink_path = temp_dir.path().join("link");
        create_temp_symlink(Path::new("sub"), &symlink_path);
        let options = DuTraversalOptions {
            all: true,
            separate_dirs: false,
            one_file_system: false,
            dereference: DuDeref::None,
            count_links: false,
            verbose: false,
            excludes: Vec::new(),
        };

        let stat = DuStat::new(&symlink_path, &options, true).unwrap();

        assert!(!stat.is_dir);
        assert_eq!(
            stat.size,
            fs::symlink_metadata(&symlink_path).unwrap().len()
        );
    }

    #[test]
    fn test_stat_new_dereferences_args_only_at_command_line_root() {
        let temp_dir = TempDir::new().unwrap();
        fs::create_dir(temp_dir.path().join("target")).unwrap();
        let symlink_path = temp_dir.path().join("link");
        create_temp_symlink(Path::new("target"), &symlink_path);
        let options = DuTraversalOptions {
            all: true,
            separate_dirs: false,
            one_file_system: false,
            dereference: DuDeref::Args(vec![symlink_path.clone()]),
            count_links: false,
            verbose: false,
            excludes: Vec::new(),
        };

        let root_stat = DuStat::new(&symlink_path, &options, true).unwrap();
        let child_stat = DuStat::new(&symlink_path, &options, false).unwrap();

        assert!(root_stat.is_dir);
        assert!(!child_stat.is_dir);
    }

    #[test]
    fn test_div_ceil() {
        assert_eq!(du_div_ceil(10, 3), 4);
        assert_eq!(du_div_ceil(15, 5), 3);
        assert_eq!(du_div_ceil(8, 2), 4);
        assert_eq!(du_div_ceil(21, 7), 3);
        assert_eq!(du_div_ceil(0, 5), 0);
    }

    #[test]
    fn test_ct_get_max_depth_zeros() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = [ctcore::ct_util_name(), dir, "--max-depth", "0"];

        // 从命令行参数中解析匹配项
        let matches = ct_app()
            .try_get_matches_from(args.iter().map(OsString::from))
            .unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        assert_eq!(max_depth.unwrap(), 0);
    }

    #[test]
    fn test_ct_get_max_depth_1() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = [ctcore::ct_util_name(), dir, "--max-depth", "1"];
        // 从命令行参数中解析匹配项
        let matches = ct_app()
            .try_get_matches_from(args.iter().map(OsString::from))
            .unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        assert_eq!(max_depth.unwrap(), 1);
    }

    #[test]
    fn test_ct_get_max_depth_2() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = [ctcore::ct_util_name(), dir, "--max-depth", "2"];
        // 从命令行参数中解析匹配项
        let matches = ct_app()
            .try_get_matches_from(args.iter().map(OsString::from))
            .unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        assert_eq!(max_depth.unwrap(), 2);
    }

    #[test]
    fn test_ct_get_max_depth_3() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = [ctcore::ct_util_name(), dir, "--max-depth", "3"];
        // 从命令行参数中解析匹配项
        let matches = ct_app()
            .try_get_matches_from(args.iter().map(OsString::from))
            .unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        assert_eq!(max_depth.unwrap(), 3);
    }

    #[test]
    fn test_ct_get_max_depth_4() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = [ctcore::ct_util_name(), dir, "--max-depth", "4"];
        // 从命令行参数中解析匹配项
        let matches = ct_app()
            .try_get_matches_from(args.iter().map(OsString::from))
            .unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        assert_eq!(max_depth.unwrap(), 4);
    }

    #[test]
    fn test_ct_get_max_depth_5() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = [ctcore::ct_util_name(), dir, "--max-depth", "5"];
        // 从命令行参数中解析匹配项
        let matches = ct_app()
            .try_get_matches_from(args.iter().map(OsString::from))
            .unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        assert_eq!(max_depth.unwrap(), 5);
    }

    #[test]
    fn test_ct_get_max_depth_negative_is_unlimited() {
        assert_eq!(du_parse_depth(Some("-1"), false).unwrap(), None);
        assert_eq!(du_parse_depth(Some("-2"), false).unwrap(), None);
        assert_eq!(du_parse_depth(Some("-0"), false).unwrap(), Some(0));
    }

    #[test]
    fn test_ct_get_max_depth_base_zero_syntax() {
        assert_eq!(du_parse_depth(Some("+1"), false).unwrap(), Some(1));
        assert_eq!(du_parse_depth(Some(" 1"), false).unwrap(), Some(1));
        assert_eq!(du_parse_depth(Some("0x1"), false).unwrap(), Some(1));
        assert_eq!(du_parse_depth(Some("010"), false).unwrap(), Some(8));
        assert!(du_parse_depth(Some("08"), false).is_err());
        assert!(du_parse_depth(Some("1 "), false).is_err());
    }

    #[test]
    fn test_ct_get_max_depth_rejects_out_of_range_values() {
        let too_large = (isize::MAX as i128 + 1).to_string();
        let too_small = (i64::MIN as i128 - 1).to_string();

        assert!(du_parse_depth(Some(&too_large), false).is_err());
        assert!(du_parse_depth(Some(&too_small), false).is_err());
    }

    #[test]
    fn test_ct_get_summarize_max_depth_5() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = [
            ctcore::ct_util_name(),
            dir,
            "--summarize",
            "--max-depth",
            "5",
        ]; //du: summarizing conflicts with --max-depth=1

        let matches = ct_app()
            .try_get_matches_from(args.iter().map(OsString::from))
            .unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize);

        let s = SummarizeDepthConflict("5".to_string()).to_string();

        assert_eq!(max_depth.unwrap_err().to_string(), s);
    }

    #[test]
    fn test_ct_fn_read_files_from() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let expected: Vec<PathBuf> = vec!["aaaa.\nbbbb.\ncccc.\ndddd.\n".into()];

        let args = vec![ctcore::ct_util_name(), dir, "--files0-from", filename];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 处理输入文件列表
        if let Some(file_from) = matches.get_one::<String>(opt_flags::FILES0_FROM) {
            let result = du_read_files_from(file_from).unwrap();
            assert_eq!(result, expected);
        };
    }

    #[cfg(unix)]
    #[test]
    fn test_ct_fn_read_files_from_preserves_non_utf8_bytes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let list = temp_dir.path().join("list");
        fs::write(&list, b"raw\xff\0ascii\0").unwrap();

        let paths = du_read_files_from(list.to_str().unwrap()).unwrap();

        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].as_os_str().as_bytes(), b"raw\xff");
        assert_eq!(paths[1], PathBuf::from("ascii"));
    }

    #[test]
    fn test_get_time_ctime() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=ctime"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let time = du_get_time(&matches);

        assert_eq!(time.unwrap(), DuTime::Changed);
    }

    #[test]
    fn test_get_time_status() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=status"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let time = du_get_time(&matches);

        assert_eq!(time.unwrap(), DuTime::Changed);
    }

    #[test]
    fn test_get_time_access() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=access"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let time = du_get_time(&matches);

        assert_eq!(time.unwrap(), DuTime::Accessed);
    }

    #[test]
    fn test_get_time_atime() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=atime"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let time = du_get_time(&matches);

        assert_eq!(time.unwrap(), DuTime::Accessed);
    }

    #[test]
    fn test_get_time_use() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=use"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let time = du_get_time(&matches);

        assert_eq!(time.unwrap(), DuTime::Accessed);
    }

    #[test]
    fn test_get_time_birth() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=birth"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let time = du_get_time(&matches);

        assert_eq!(time.unwrap(), DuTime::Created);
    }

    #[test]
    fn test_get_time_creation() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=creation"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let time = du_get_time(&matches);

        assert_eq!(time.unwrap(), DuTime::Created);
    }

    #[test]
    fn test_get_time_style_full_iso() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time-style", "full-iso"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let time = du_get_time(&matches);

        assert_eq!(time, None);
    }

    #[test]
    fn test_get_time_style_iso() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time-style", "iso"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let time = du_get_time(&matches);

        assert_eq!(time, None);
    }

    #[test]
    fn test_parse_time_style_custom_format() {
        assert_eq!(
            du_parse_time_style(Some("+%Y-%m-%d %H:%M:%S %z (%Z)")).unwrap(),
            "+%Y-%m-%d %H:%M:%S %z (%Z)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_du_format_local_time_strftime_percent_z() {
        let out = du_format_local_time_strftime(0, 0, "%Z").unwrap();
        assert!(!out.is_empty());
        assert!(!out.contains(':'));
    }

    #[cfg(unix)]
    #[test]
    fn test_du_format_local_time_strftime_percent_n() {
        assert_eq!(
            du_format_local_time_strftime(0, 123_456_789, "%s.%N %%N").as_deref(),
            Some("0.123456789 %N")
        );
    }

    // 用于同步访问 TIME_STYLE 环境变量的全局锁
    static TEST_TIME_STYLE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(unix)]
    #[test]
    fn test_time_style_from_env_when_time_enabled() {
        let _guard = TEST_TIME_STYLE_LOCK
            .lock()
            .expect("failed to lock TIME_STYLE test mutex");
        let saved_time_style = std::env::var("TIME_STYLE").ok();

        unsafe {
            std::env::set_var("TIME_STYLE", "iso");
        }

        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let args = vec![ctcore::ct_util_name(), dir, "--time"];
        let matches = ct_app().try_get_matches_from(args).unwrap();
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);
        let max_depth = du_get_max_depth(&matches, summarize).unwrap();
        let time = du_get_time(&matches);
        let size_format = du_get_size_format(&matches).unwrap();
        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format).unwrap();

        assert_eq!(stat_printer.time_format, "%Y-%m-%d");

        unsafe {
            match saved_time_style.as_deref() {
                Some(v) => std::env::set_var("TIME_STYLE", v),
                None => std::env::remove_var("TIME_STYLE"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_time_style_ignored_without_time() {
        let _guard = TEST_TIME_STYLE_LOCK
            .lock()
            .expect("failed to lock TIME_STYLE test mutex");
        let saved_time_style = std::env::var("TIME_STYLE").ok();

        unsafe {
            std::env::set_var("TIME_STYLE", "invalid-style");
        }

        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let args = vec![ctcore::ct_util_name(), dir];
        let matches = ct_app().try_get_matches_from(args).unwrap();
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);
        let max_depth = du_get_max_depth(&matches, summarize).unwrap();
        let time = du_get_time(&matches);
        let size_format = du_get_size_format(&matches).unwrap();

        assert!(du_get_stat_printer(&matches, summarize, max_depth, time, size_format).is_ok());

        unsafe {
            match saved_time_style.as_deref() {
                Some(v) => std::env::set_var("TIME_STYLE", v),
                None => std::env::remove_var("TIME_STYLE"),
            }
        }
    }

    #[test]
    fn test_ct_get_size_format_human_readable() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-h"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let size_format = du_get_size_format(&matches).unwrap();

        assert_eq!(size_format, DuSizeFormat::Human(1024));
    }

    #[test]
    fn test_ct_get_size_format_human_readable_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--human-readable"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let size_format = du_get_size_format(&matches).unwrap();

        assert_eq!(size_format, DuSizeFormat::Human(1024));
    }

    #[test]
    fn test_ct_get_size_format_si() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--si"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let size_format = du_get_size_format(&matches).unwrap();

        assert_eq!(size_format, DuSizeFormat::Human(1000));
    }

    #[test]
    fn test_ct_get_size_format_bytes() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--bytes"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let size_format = du_get_size_format(&matches).unwrap();

        assert_eq!(size_format, BlockSize(1));
    }

    #[test]
    fn test_ct_get_size_format_block_size_1k() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size=1k"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let size_format = du_get_size_format(&matches).unwrap();

        assert_eq!(size_format, BlockSize(1024));
    }

    #[test]
    fn test_ct_get_size_format_block_size_1m() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size=1m"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let size_format = du_get_size_format(&matches).unwrap();

        assert_eq!(size_format, BlockSize(1048576));
    }

    #[test]
    fn test_ct_get_size_format_block_size_1k_c() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size=1K"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let size_format = du_get_size_format(&matches).unwrap();

        assert_eq!(size_format, BlockSize(1024));
    }

    #[test]
    fn test_ct_get_size_format_block_size_1m_c() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_main_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size=1M"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        let size_format = du_get_size_format(&matches).unwrap();

        assert_eq!(size_format, BlockSize(1048576));
    }
}

#[cfg(test)]
mod test_ct_stat_printer {
    use super::*;

    use std::io::Write;
    use tempfile::Builder;

    #[test]
    fn test_stat_printer_a() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir];

        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_all() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--all"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--apparent-size"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_all_apparent_size() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--all", "--apparent-size"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_block_size_1k() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1K"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_block_size_1m() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1M"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_block_size_1g() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1g"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_block_size_1t() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1T"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_block_size_1p() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1P"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_block_size_1e() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1E"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_block_size_1z() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1Z"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = DuSizeFormat::Human(1024);

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_block_size_1y() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--block-size", "1Y"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = DuSizeFormat::Human(1024);

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_block_size_1k() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--apparent-size",
            "--block-size",
            "1K",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_block_size_1m() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--apparent-size",
            "--block-size",
            "1M",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_block_size_1g() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--apparent-size",
            "--block-size",
            "1g",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_block_size_1t() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--apparent-size",
            "--block-size",
            "1T",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_block_size_1p() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--apparent-size",
            "--block-size",
            "1P",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_block_size_1e() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--apparent-size",
            "--block-size",
            "1E",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_block_size_1z() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--apparent-size",
            "--block-size",
            "1Z",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = DuSizeFormat::Human(1024);

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_block_size_1y() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--apparent-size",
            "--block-size",
            "1Y",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = DuSizeFormat::Human(1024);

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_bk() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-BK"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_bm() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-BM"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_bg() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-BG"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_bt() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-BT"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_bp() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-BP"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_be() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-BE"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_bz() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-BZ"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = DuSizeFormat::Human(1024);

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_by() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-BY"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = DuSizeFormat::Human(1024);

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_bk() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--apparent-size", "-BK"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_bm() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--apparent-size", "-BM"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_bg() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--apparent-size", "-BG"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_bt() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--apparent-size", "-BT"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_bp() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--apparent-size", "-BP"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_be() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--apparent-size", "-BE"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_apparent_size_bz() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--apparent-size", "-BZ"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = DuSizeFormat::Human(1024);

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);

        //du: --block-size argument 'Z' too large
    }

    #[test]
    fn test_stat_printer_apparent_size_by() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--apparent-size", "-BY"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = DuSizeFormat::Human(1024);

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);

        //du: --block-size argument 'Y' too large
    }

    #[test]
    fn test_stat_printer_b() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-b"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_bytes() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--bytes"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_total() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-c"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_total_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file).unwrap();
        let _ = test_file.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--total"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_block_size_flag_k() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-k"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_block_size_flag_1m() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-m"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_max_depth_zeros() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "0"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_max_depth_1() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "1"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_max_depth_2() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "2"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_max_depth_3() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "3"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_max_depth_4() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "4"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_max_depth_5() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--max-depth", "5"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_d_zeros() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-d", "0"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_d_1() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-d", "1"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_d_2() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-d", "2"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_d_3() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-d", "3"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_d_4() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-d", "4"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_d_5() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-d", "5"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_human_readable() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-h"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_human_readable_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--human-readable"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_inodes() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--inodes"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_human_readable_inodes() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--human-readable", "--inodes"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_k() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-k"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_count_links() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-l"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_count_links_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--count-links"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_dereference() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-L"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_dereference_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--dereference"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_dereference_args() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-D"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_dereference_args_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--dereference-args"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_no_dereference() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-P"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_no_dereference_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--no-dereference"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_m() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-m"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_null() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--null"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_zero() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-0"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_separate_dirs() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-S"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_separate_dirs_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--separate-dirs"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_summarize() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-s"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_summarize_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--summarize"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_si() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--si"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_summarize_si() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-s", "--si"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_summarize_si_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--summarize", "--si"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_one_file_system() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-x"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_one_file_system_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--one-file-system"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_threshold_1k() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1KB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_threshold_1m() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1MB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_threshold_1g() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1GB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_threshold_1t() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1TB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_threshold_1p() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1PB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_threshold_1e() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1EB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_threshold_1y() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1MB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_threshold_1z() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--threshold", "1GB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_t_1k() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-t", "1KB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_t_1m() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-t", "1MB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_t_1g() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-t", "1GB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_t_1t() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-t", "1TB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_t_1p() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-t", "1PB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_t_1e() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-t", "1EB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_t_1y() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-t", "1KB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = DuSizeFormat::Human(1024);

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_t_1z() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-t", "1KB"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = DuSizeFormat::Human(1024);

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_verbose() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-v"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_verbose_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--verbose"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--exclude", filename];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_verbose_exclude() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--verbose",
            "--exclude",
            filename,
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_x_from() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "-X", filename];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--exclude-from", filename];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_files0_from() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--files0-from", filename];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_files0_from() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            filename,
            "--files0-from",
            filename,
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_time_ctime() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=ctime"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_time_status() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=status"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_time_access() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=access"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_time_atime() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=atime"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_time_use() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=use"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_time_birth() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=birth"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_time_creation() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time=creation"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_time_ctime() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            filename,
            "--time=ctime",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_time_status() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            filename,
            "--time=status",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_time_access() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            filename,
            "--time=access",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_time_atime() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            filename,
            "--time=atime",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_time_use() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            filename,
            "--time=use",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_time_birth() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            filename,
            "--time=birth",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_time_creation() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let filename = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            filename,
            "--time=creation",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_time_style_full_iso() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time-style", "full-iso"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_time_style_long_iso() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time-style", "long-iso"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_time_style_iso() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![ctcore::ct_util_name(), dir, "--time-style", "iso"];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_time_style_full_iso() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let file_name = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            file_name,
            "--time-style",
            "full-iso",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_time_style_long_iso() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let file_name = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            file_name,
            "--time-style",
            "long-iso",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_time_style_iso() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let file_name = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            file_name,
            "--time-style",
            "iso",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }

    #[test]
    fn test_stat_printer_exclude_from_files0_from_time_time_style_iso() {
        let temp_dir = Builder::new()
            .prefix("tests_stat_printer_dir")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file.txt");
        let mut file = File::create(&test_file_1).unwrap();
        let file_name = test_file_1.to_str().unwrap();
        let dir = temp_dir.path().to_str().unwrap();
        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let args = vec![
            ctcore::ct_util_name(),
            dir,
            "--exclude-from",
            file_name,
            "--files0-from",
            file_name,
            "--time=ctime",
            "--time-style",
            "iso",
        ];
        // 从命令行参数中解析匹配项
        let matches = ct_app().try_get_matches_from(args).unwrap();

        // 解析是否需要汇总信息
        let summarize = matches.get_flag(opt_flags::SUMMARIZE);

        let max_depth = du_get_max_depth(&matches, summarize).unwrap();

        let time = du_get_time(&matches);

        let size_format = du_get_size_format(&matches).unwrap();

        let stat_printer =
            du_get_stat_printer(&matches, summarize, max_depth, time, size_format.clone()).unwrap();
        assert_eq!(stat_printer.max_depth, max_depth);
        assert_eq!(stat_printer.time, time);
        assert_eq!(stat_printer.size_format, size_format);
        assert_eq!(stat_printer.summarize, summarize);
    }
}
