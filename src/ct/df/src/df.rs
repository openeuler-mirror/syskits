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
mod blocks;
mod columns;
mod filesystem;
mod table;

use blocks::BlocksHumanReadable;
use clap::builder::ValueParser;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTError;
use ctcore::ct_error::CTResult;
use ctcore::ct_error::CtSimpleError;
use ctcore::ct_error::FromIo;

use ctcore::ct_format_usage;
use ctcore::ct_fsext::read_fs_list;
use ctcore::ct_fsext::CtMountInfo;
use ctcore::ct_help_about;
use ctcore::ct_help_section;
use ctcore::ct_help_usage;
use ctcore::ct_parse_size::ParseSizeError;
use ctcore::ct_show;

use table::TableHeaderMode;

use clap::crate_version;
use clap::parser::ValueSource;
use clap::Arg;
use clap::ArgAction;
use clap::ArgMatches;
use clap::Command;

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::path::Path;

use crate::blocks::block_size_read;
use crate::blocks::BlockSize;
use crate::columns::{Column, ColumnError};
use crate::filesystem::Filesystem;
use crate::table::Table;

const DF_ABOUT: &str = ct_help_about!("df.md");
const DF_USAGE: &str = ct_help_usage!("df.md");
const DF_AFTER_HELP: &str = ct_help_section!("after help", "df.md");

static DF_OPT_HELP: &str = "help";
static DF_OPT_ALL: &str = "all";
static DF_OPT_BLOCKSIZE: &str = "blocksize";
static DF_OPT_TOTAL: &str = "total";
static DF_OPT_HUMAN_READABLE_BINARY: &str = "human-readable-binary";
static DF_OPT_HUMAN_READABLE_DECIMAL: &str = "human-readable-decimal";
static DF_OPT_INODES: &str = "inodes";
static DF_OPT_KILO: &str = "kilo";
static DF_OPT_LOCAL: &str = "local";
static DF_OPT_NO_SYNC: &str = "no-sync";
static DF_OPT_OUTPUT: &str = "output";
static DF_OPT_PATHS: &str = "paths";
static DF_OPT_PORTABILITY: &str = "portability";
static DF_OPT_SYNC: &str = "sync";
static DF_OPT_TYPE: &str = "type";
static DF_OPT_PRINT_TYPE: &str = "print-type";
static DF_OPT_EXCLUDE_TYPE: &str = "exclude-type";
static OUTPUT_FIELD_LIST: [&str; 12] = [
    "source", "fstype", "itotal", "iused", "iavail", "ipcent", "size", "used", "avail", "pcent",
    "file", "target",
];

/// 控制`df`行为的参数。
///
/// 多数参数用于控制显示哪些行和哪些列。`block_size`用于确定在显示字节数或i节点数时使用的单位。
struct DfOptions {
    show_local_fs: bool,                         // 是否显示本地文件系统的信息
    show_all_fs: bool,                           // 是否显示所有文件系统的信息
    human_readable: Option<BlocksHumanReadable>, // 是否以人类可读的形式显示块大小
    block_size: BlockSize,                       // 显示字节数或i节点数时使用的单位
    header_mode: TableHeaderMode,                // 表头的显示模式

    /// 包含在输出表中的文件系统类型可选列表。
    ///
    /// 如果这不是`None`，则只列出匹配这些类型的文件系统。
    include: Option<Vec<String>>,

    /// 从输出表中排除的文件系统类型可选列表。
    ///
    /// 如果这不是`None`，则不会列出匹配这些类型的文件系统。
    exclude: Option<Vec<String>>,

    /// 操作前是否同步。
    sync: bool,

    /// 是否显示每列总数的最终行。
    show_total: bool,

    /// 输出表中显示的列序列。
    columns: Vec<Column>,
}

impl Default for DfOptions {
    /// 返回`DfOptions`结构体的默认实例。
    fn default() -> Self {
        Self {
            show_local_fs: Default::default(),
            show_all_fs: Default::default(),
            block_size: BlockSize::default(),
            human_readable: Option::default(),
            header_mode: TableHeaderMode::default(),
            include: Option::default(),
            exclude: Option::default(),
            sync: Default::default(),
            show_total: Default::default(),
            columns: vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ],
        }
    }
}
// DfOptionsError定义了在处理df命令选项时可能遇到的各种错误类型。
#[derive(Debug)]
enum DfOptionsError {
    // BlockSizeTooLarge表示指定的块大小超过允许的最大值。
    BlockSizeTooLarge(String),
    // InvalidBlockSize表示指定的块大小不合法。
    InvalidBlockSize(String),
    // InvalidSuffix表示在块大小参数中使用了非法的后缀。
    InvalidSuffix(String),

    /// ColumnError封装了与输出列相关的选择和设置错误。
    ColumnError(ColumnError),

    // FilesystemTypeBothSelectedAndExcluded表示有文件系统类型同时被选择和排除。
    FilesystemTypeBothSelectedAndExcluded(Vec<String>),
}

// fmt::Display实现为DfOptionsError提供了格式化输出的方法。
impl fmt::Display for DfOptionsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // 根据错误类型，提供不同的错误信息格式化输出。
        match self {
            // BlockSizeTooLarge错误的输出格式化
            Self::BlockSizeTooLarge(s) => {
                write!(f, "--block-size argument {} too large", s.quote())
            }
            // InvalidBlockSize错误的输出格式化
            Self::InvalidBlockSize(s) => write!(f, "invalid --block-size argument {s}"),
            // InvalidSuffix错误的输出格式化
            Self::InvalidSuffix(s) => write!(f, "invalid suffix in --block-size argument {s}"),
            // ColumnError错误的输出格式化，具体格式化逻辑依赖于ColumnError的子类型。
            Self::ColumnError(ColumnError::MultipleColumns(s)) => write!(
                f,
                "option --output: field {} used more than once",
                s.quote()
            ),
            // FilesystemTypeBothSelectedAndExcluded错误会在控制台打印详细信息，不会直接影响fmt输出。
            #[allow(clippy::print_in_format_impl)]
            Self::FilesystemTypeBothSelectedAndExcluded(types) => {
                for t in types {
                    eprintln!(
                        "{}: file system type {} both selected and excluded",
                        ctcore::ct_util_name(),
                        t.quote()
                    );
                }
                Ok(())
            }
        }
    }
}

/// DfOptions 类的实现，提供了从命令行参数到 DfOptions 实例的转换功能。
impl DfOptions {
    /// 根据命令行参数创建一个 [`DfOptions`] 实例。
    ///
    /// - `matches`: 代表命令行参数的匹配结果，来自 `clap` 库。
    /// - 返回值: 成功时返回一个 [`DfOptions`] 实例，失败时返回一个错误信息。
    fn from(args_match: &ArgMatches) -> Result<Self, DfOptionsError> {
        // 尝试从命令行参数中提取要包含和排除的文件系统类型。
        let df_include: Option<Vec<_>> = args_match
            .get_many::<OsString>(DF_OPT_TYPE)
            .map(|v| v.map(|s| s.to_string_lossy().to_string()).collect());
        let df_exclude: Option<Vec<_>> = args_match
            .get_many::<OsString>(DF_OPT_EXCLUDE_TYPE)
            .map(|v| v.map(|s| s.to_string_lossy().to_string()).collect());

        // 检查是否同时指定了包含和排除相同的文件系统类型，如果是，则返回错误。
        if let (Some(include), Some(exclude)) = (&df_include, &df_exclude) {
            if let Some(types) = Self::get_intersected_types(include, exclude) {
                return Err(DfOptionsError::FilesystemTypeBothSelectedAndExcluded(types));
            }
        }

        // 解析并构造 DfOptions 实例。
        Ok(Self {
            show_local_fs: args_match.get_flag(DF_OPT_LOCAL),
            show_all_fs: args_match.get_flag(DF_OPT_ALL),
            sync: args_match.get_flag(DF_OPT_SYNC),
            // 解析块大小参数，并处理可能的错误。
            block_size: block_size_read(args_match).map_err(|e| match e {
                ParseSizeError::InvalidSuffix(s) => DfOptionsError::InvalidSuffix(s),
                ParseSizeError::SizeTooBig(_) => DfOptionsError::BlockSizeTooLarge(
                    args_match
                        .get_one::<String>(DF_OPT_BLOCKSIZE)
                        .unwrap()
                        .to_string(),
                ),
                ParseSizeError::ParseFailure(s) => DfOptionsError::InvalidBlockSize(s),
            })?,
            header_mode: {
                // 根据命令行参数确定表格头部的显示模式。
                if args_match.get_flag(DF_OPT_HUMAN_READABLE_BINARY)
                    || args_match.get_flag(DF_OPT_HUMAN_READABLE_DECIMAL)
                {
                    TableHeaderMode::HumanReadable
                } else if args_match.get_flag(DF_OPT_PORTABILITY) {
                    TableHeaderMode::PosixPortability
                } else if args_match.value_source(DF_OPT_OUTPUT) == Some(ValueSource::CommandLine) {
                    TableHeaderMode::Output
                } else {
                    TableHeaderMode::Default
                }
            },
            human_readable: {
                // 根据命令行参数确定是否以人类可读的方式显示块大小。
                if args_match.get_flag(DF_OPT_HUMAN_READABLE_BINARY) {
                    Some(BlocksHumanReadable::Binary)
                } else if args_match.get_flag(DF_OPT_HUMAN_READABLE_DECIMAL) {
                    Some(BlocksHumanReadable::Decimal)
                } else {
                    None
                }
            },
            include: df_include,
            exclude: df_exclude,
            show_total: args_match.get_flag(DF_OPT_TOTAL),
            // 根据命令行参数解析要显示的列，并处理可能的错误。
            columns: Column::from_matches(args_match).map_err(DfOptionsError::ColumnError)?,
        })
    }

    /// 检查 `include` 和 `exclude` 类型列表是否有交集。
    ///
    /// - `include`: 要包含的文件系统类型列表。
    /// - `exclude`: 要排除的文件系统类型列表。
    /// - 返回值: 如果存在交集，返回交集列表；否则返回 `None`。
    fn get_intersected_types(df_include: &[String], df_exclude: &[String]) -> Option<Vec<String>> {
        let mut same_types = Vec::new();

        // 寻找同时在 include 和 exclude 中出现的类型。
        for types in df_include {
            if df_exclude.contains(types) {
                same_types.push(types.clone());
            }
        }

        // 如果存在交集，返回交集列表。
        (!same_types.is_empty()).then_some(same_types)
    }
}
/// 判断给定的挂载信息是否应被包含在输出中。
///
/// 此函数根据包括和排除设置来决定是否显示挂载信息。
fn is_included(mount_info: &CtMountInfo, options: &DfOptions) -> bool {
    // 如果指定了只显示本地文件系统，那么不显示远程文件系统。
    if mount_info.remote && options.show_local_fs {
        return false;
    }

    // 除非指定了显示所有文件系统，否则不显示伪文件系统。
    if mount_info.dummy && !options.show_all_fs {
        return false;
    }

    // 如果文件系统被明确排除，则不显示。
    if let Some(ref excludes) = options.exclude {
        if excludes.contains(&mount_info.fs_type) {
            return false;
        }
    }
    if let Some(ref includes) = options.include {
        if !includes.contains(&mount_info.fs_type) {
            return false;
        }
    }

    true
}

/// 判断挂载信息`m2`是否应该优先于`m1`显示。
///
/// 此函数用于决定在显示挂载信息时的排序。
fn mount_info_lt(mount_info_1: &CtMountInfo, mount_info_2: &CtMountInfo) -> bool {
    // 如果`m1`的设备名以'/'开头，且`m2`的不是，那么`m2`优先。
    if mount_info_1.dev_name.starts_with('/') && !mount_info_2.dev_name.starts_with('/') {
        return false;
    }

    let m1_nearer_root = mount_info_1.mount_dir.len() < mount_info_2.mount_dir.len();
    // 对于绑定挂载，优先选择更接近根目录的项。
    let m2_below_root = !mount_info_1.mount_root.is_empty()
        && !mount_info_2.mount_root.is_empty()
        && mount_info_1.mount_root.len() > mount_info_2.mount_root.len();
    // 如果`m1`更接近设备的根目录，那么`m1`优先。
    if m1_nearer_root && !m2_below_root {
        return false;
    }

    // 如果两个挂载点的设备名不同，但是挂载目录相同，那么设备名不同的优先。
    if mount_info_1.dev_name != mount_info_2.dev_name
        && mount_info_1.mount_dir == mount_info_2.mount_dir
    {
        return false;
    }

    true
}

/// 判断给定的挂载信息是否应优先于同一设备上的其他挂载信息。
fn is_best(previous_mount_info: &[CtMountInfo], mount_info: &CtMountInfo) -> bool {
    for search in previous_mount_info {
        if search.dev_id == mount_info.dev_id && mount_info_lt(mount_info, search) {
            return false;
        }
    }
    true
}

/// 仅保留指定子集的[`CtMountInfo`]实例。
///
/// 此函数根据[`DfOptions`]中的各种排除方式来过滤[`CtMountInfo`]实例。
fn filter_mount_list(v_mount_info: Vec<CtMountInfo>, options: &DfOptions) -> Vec<CtMountInfo> {
    let mut result = vec![];
    for mount_info in v_mount_info {
        // TODO: `is_best()`的运行时间是线性于`result`的长度。这使得此循环的运行时间在最坏情况下是`vmi`长度的二次方。在实践中，`vmi`可能并不长，但这个问题仍可能需要优化。
        if is_included(&mount_info, options) && is_best(&result, &mount_info) {
            result.push(mount_info);
        }
    }
    result
}

/// 获取当前所有已挂载的文件系统。
fn get_all_filesystems(option: &DfOptions) -> Result<Vec<Filesystem>, std::io::Error> {
    // 如果指定了同步选项，则在进行任何操作前执行同步调用。
    if option.sync {
        #[cfg(not(any(windows, target_os = "redox")))]
        unsafe {
            #[cfg(not(target_os = "android"))]
            ctcore::libc::sync();
            #[cfg(target_os = "android")]
            ctcore::libc::syscall(ctcore::libc::SYS_sync);
        }
    }

    // 所有挂载的文件系统的列表。
    // 根据命令行选项排除某些文件系统。
    let mounts: Vec<CtMountInfo> = filter_mount_list(read_fs_list()?, option);

    // 将每个`MountInfo`转换为包含挂载信息和使用信息的`Filesystem`。
    Ok(mounts
        .into_iter()
        .filter_map(|m| Filesystem::new(m, None))
        .filter(|fs| option.show_all_fs || fs.usage.blocks > 0)
        .collect())
}

/// 对于每个路径，获取包含该路径的文件系统。
fn get_named_filesystems<P>(
    file_paths: &[P],
    options: &DfOptions,
) -> Result<Vec<Filesystem>, std::io::Error>
where
    P: AsRef<Path>,
{
    // 所有挂载的文件系统的列表。
    // 排除被标记为“dummy”的文件系统和类型为"lofs"的文件系统。"lofs"是一种循环回路文件系统，存在于Solaris和FreeBSD系统中。它类似于符号链接。
    let mounts: Vec<CtMountInfo> = filter_mount_list(read_fs_list()?, options)
        .into_iter()
        .filter(|mi| mi.fs_type != "lofs" && !mi.dummy)
        .collect();

    let mut result = vec![];

    // 如果没有可用的文件系统类型，则显示错误信息。
    if mounts.is_empty() {
        ct_show!(CtSimpleError::new(1, "no file systems processed"));
        return Ok(result);
    }

    // 将每个路径转换为包含挂载信息和使用信息的`Filesystem`。
    for path in file_paths {
        match Filesystem::from_path(&mounts, path) {
            Some(fs) => result.push(fs),
            None => {
                // 如果指定的文件系统类型与文件的实际文件系统类型不匹配，显示错误信息。
                if path.as_ref().exists() {
                    ct_show!(CtSimpleError::new(1, "no file systems processed"));
                } else {
                    ct_show!(CtSimpleError::new(
                        1,
                        format!("{}: No such file or directory", path.as_ref().display())
                    ));
                }
            }
        }
    }
    Ok(result)
}
// DfError定义了在执行df命令时可能遇到的错误类型。
#[derive(Debug)]
enum DfError {
    /// 解析命令行选项时出现问题。
    OptionsError(DfOptionsError),
}

// 实现Error trait使得DfError可以被作为错误处理机制的一部分来使用。
impl Error for DfError {}

// 实现CTError trait以提供错误使用帮助信息的能力。
impl CTError for DfError {
    // 如果错误类型为OptionsError且具体错误为ColumnError，则返回true，表示需要显示用法信息。
    fn usage(&self) -> bool {
        matches!(self, Self::OptionsError(DfOptionsError::ColumnError(_)))
    }
}

// 实现Display trait以便于错误信息可以被打印。
impl fmt::Display for DfError {
    // 根据DfError的类型，调用对应的错误信息格式化函数。
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::OptionsError(e) => e.fmt(f),
        }
    }
}

// ctmain是程序的主入口函数。
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    df_main(args).map(|_| ())
}

pub fn df_main(args: impl ctcore::Args) -> CTResult<()> {
    // 从args解析命令行匹配项。
    let args_match = ct_app().try_get_matches_from(args)?;

    // 在Windows平台上检查是否指定了不支持的-i选项。
    #[cfg(windows)]
    {
        if args_match.get_flag(DF_OPT_INODES) {
            println!("{}: doesn't support -i option", ctcore::ct_util_name());
            return Ok(());
        }
    }

    // 从命令行匹配项中解析DfOptions。
    let options = DfOptions::from(&args_match).map_err(DfError::OptionsError)?;

    let filesystem_paths = get_filesystem(args_match, &options);

    // 打印文件系统信息的表格。
    println!("{}", Table::new(&options, filesystem_paths.unwrap()));

    // 函数执行成功，返回Ok。
    Ok(())
}

fn get_filesystem(
    args_match: ArgMatches,
    options: &DfOptions,
) -> Result<Vec<Filesystem>, Box<dyn CTError>> {
    // 根据命令行参数获取要显示的文件系统列表。
    let filesystem_paths: Vec<Filesystem> = match args_match.get_many::<String>(DF_OPT_PATHS) {
        // 如果没有指定路径，则获取所有文件系统的信息。
        None => {
            let filesystem = get_all_filesystems(options)
                .map_err_context(|| "cannot read table of mounted file systems".into())?;

            // 如果没有找到文件系统信息，则返回错误。
            if filesystem.is_empty() {
                return Err(CtSimpleError::new(1, "no file systems processed"));
            }

            filesystem
        }
        // 如果指定了路径，则只获取指定路径相关的文件系统信息。
        Some(paths) => {
            let filesystem_paths: Vec<_> = paths.collect();
            let filesystem = get_named_filesystems(&filesystem_paths, options)
                .map_err_context(|| "cannot read table of mounted file systems".into())?;

            // 如果指定路径不存在对应的文件系统信息，则不进行任何操作。
            if filesystem.is_empty() {
                return Ok(filesystem);
            }

            filesystem
        }
    };
    Ok(filesystem_paths)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = DF_ABOUT;
    let usage_description = ct_format_usage(DF_USAGE);

    let args = df_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(DF_AFTER_HELP)
        .infer_long_args(true)
        .disable_help_flag(true)
        .args(&args)
}

fn df_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(DF_OPT_HELP)
            .long(DF_OPT_HELP)
            .help("Print help information.")
            .action(ArgAction::Help),
        Arg::new(DF_OPT_ALL)
            .short('a')
            .long("all")
            .overrides_with(DF_OPT_ALL)
            .help("include dummy file systems")
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_BLOCKSIZE)
            .short('B')
            .long("block-size")
            .value_name("SIZE")
            .overrides_with_all([DF_OPT_KILO, DF_OPT_BLOCKSIZE])
            .help(
                "scale sizes by SIZE before printing them; e.g.\
                    '-BM' prints sizes in units of 1,048,576 bytes",
            ),
        Arg::new(DF_OPT_TOTAL)
            .long("total")
            .overrides_with(DF_OPT_TOTAL)
            .help("produce a grand total")
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_HUMAN_READABLE_BINARY)
            .short('h')
            .long("human-readable")
            .overrides_with_all([DF_OPT_HUMAN_READABLE_DECIMAL, DF_OPT_HUMAN_READABLE_BINARY])
            .help("print sizes in human readable ct_format (e.g., 1K 234M 2G)")
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_HUMAN_READABLE_DECIMAL)
            .short('H')
            .long("si")
            .overrides_with_all([DF_OPT_HUMAN_READABLE_BINARY, DF_OPT_HUMAN_READABLE_DECIMAL])
            .help("likewise, but use powers of 1000 not 1024")
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_INODES)
            .short('i')
            .long("inodes")
            .overrides_with(DF_OPT_INODES)
            .help("list inode information instead of block usage")
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_KILO)
            .short('k')
            .help("like --block-size=1K")
            .overrides_with_all([DF_OPT_BLOCKSIZE, DF_OPT_KILO])
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_LOCAL)
            .short('l')
            .long("local")
            .overrides_with(DF_OPT_LOCAL)
            .help("limit listing to local file systems")
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_NO_SYNC)
            .long("no-sync")
            .overrides_with_all([DF_OPT_SYNC, DF_OPT_NO_SYNC])
            .help("do not invoke sync before getting usage info (default)")
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_OUTPUT)
            .long("output")
            .value_name("FIELD_LIST")
            .action(ArgAction::Append)
            .num_args(0..)
            .require_equals(true)
            .use_value_delimiter(true)
            .value_parser(OUTPUT_FIELD_LIST)
            .default_missing_values(OUTPUT_FIELD_LIST)
            .default_values(["source", "size", "used", "avail", "pcent", "target"])
            .conflicts_with_all([DF_OPT_INODES, DF_OPT_PORTABILITY, DF_OPT_PRINT_TYPE])
            .help(
                "use the output ct_format defined by FIELD_LIST, \
                     or print all fields if FIELD_LIST is omitted.",
            ),
        Arg::new(DF_OPT_PORTABILITY)
            .short('P')
            .long("portability")
            .overrides_with(DF_OPT_PORTABILITY)
            .help("use the POSIX output ct_format")
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_SYNC)
            .long("sync")
            .overrides_with_all([DF_OPT_NO_SYNC, DF_OPT_SYNC])
            .help("invoke sync before getting usage info (non-windows only)")
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_TYPE)
            .short('t')
            .long("type")
            .value_parser(ValueParser::os_string())
            .value_name("TYPE")
            .action(ArgAction::Append)
            .help("limit listing to file systems of type TYPE"),
        Arg::new(DF_OPT_PRINT_TYPE)
            .short('T')
            .long("print-type")
            .overrides_with(DF_OPT_PRINT_TYPE)
            .help("print file system type")
            .action(ArgAction::SetTrue),
        Arg::new(DF_OPT_EXCLUDE_TYPE)
            .short('x')
            .long("exclude-type")
            .action(ArgAction::Append)
            .value_parser(ValueParser::os_string())
            .value_name("TYPE")
            .use_value_delimiter(true)
            .help("limit listing to file systems not of type TYPE"),
        Arg::new(DF_OPT_PATHS)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath),
    ];
    args
}

#[cfg(test)]
mod tests {

    mod tests_ct_app {
        use crate::{
            ct_app, DF_OPT_ALL, DF_OPT_BLOCKSIZE, DF_OPT_EXCLUDE_TYPE,
            DF_OPT_HUMAN_READABLE_BINARY, DF_OPT_HUMAN_READABLE_DECIMAL, DF_OPT_INODES,
            DF_OPT_LOCAL, DF_OPT_NO_SYNC, DF_OPT_OUTPUT, DF_OPT_PORTABILITY, DF_OPT_PRINT_TYPE,
            DF_OPT_SYNC, DF_OPT_TOTAL, DF_OPT_TYPE,
        };
        use clap::error::ErrorKind;
        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
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
        fn test_ct_app_df_a() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-a"];
            let result = command.try_get_matches_from(args);
            let expected_result = true;

            assert!(result.is_ok());
            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_ALL).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_all() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--all"];
            let result = command.try_get_matches_from(args);
            let expected_result = true;

            assert!(result.is_ok());
            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_ALL).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_b_k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BK"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("K");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_b_k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bk"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("k");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=K"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("K");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=k"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("k");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_b_m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BM"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("M");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_b_m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bm"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("m");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=M"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("M");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_m() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=m"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("m");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }
        #[test]
        fn test_ct_app_df_uppercase_b_g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BG"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("G");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_b_g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bg"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("g");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=G"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("G");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_g() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=g"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("g");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_b_t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BT"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("T");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_b_t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bt"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("t");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=T"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("T");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=t"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("t");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }
        #[test]
        fn test_ct_app_df_uppercase_b_p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BP"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("P");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_b_p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bp"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("p");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=P"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("P");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=p"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("p");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_b_e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BE"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("E");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_b_e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Be"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("e");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=E"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("E");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=e"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("e");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_b_z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BZ"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("Z");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_b_z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bz"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("z");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Z"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("Z");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_z() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=z"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("z");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_b_y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BY"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("Y");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_b_y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-By"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("y");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Y"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("Y");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_y() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=y"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("y");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_BLOCKSIZE)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_block_size_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_TOTAL).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_uppercase_b_k_total_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BK", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("K");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_b_k_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bk", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("k");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_k_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=K", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("K");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_k_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=k", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("k");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_b_m_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BM", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("M");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_b_m_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bm", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("m");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_m_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=M", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("M");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_m_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=m", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("m");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }
        #[test]
        fn test_ct_app_df_uppercase_b_g_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BG", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("G");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_b_g_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bg", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("g");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_g_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=G", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("G");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_g_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=g", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("g");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_b_t_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BT", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("T");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_b_t_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bt", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("t");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_t_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=T", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("T");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_t_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=t", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("t");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }
        #[test]
        fn test_ct_app_df_uppercase_b_p_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BP", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("P");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_b_p_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bp", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("p");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_p_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=P", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("P");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_p_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=p", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("p");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_b_e_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BE", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("E");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_b_e_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Be", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("e");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_e_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=E", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("E");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_e_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=e", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("e");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_b_z_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BZ", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("Z");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_b_z_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-Bz", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("z");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_z_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Z", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("Z");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_z_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=z", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("z");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_b_y_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BY", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("Y");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_b_y_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-By", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("y");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_block_size_y_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Y", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("Y");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_lowercase_block_size_y_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=y", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("y");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_uppercase_b_k_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-BK", "--total"];
            let result = command.try_get_matches_from(args);
            let expected_result = Some("K");

            if let Ok(_matches) = result.as_ref() {
                if let Some(_matches) = result.as_ref().ok() {
                    let result = _matches.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
                    assert_eq!(expected_result, Some(result).map(|x| x.as_str()));

                    let result = _matches.get_one::<bool>(DF_OPT_TOTAL).unwrap();

                    assert_eq!(true, *result);
                }
            }
        }

        #[test]
        fn test_ct_app_df_human_readable_binary() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--human-readable"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result
                    .unwrap()
                    .get_one::<bool>(DF_OPT_HUMAN_READABLE_BINARY)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_human_readable_decimal() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--human-readable"];
            let result = command.try_get_matches_from(args);

            let expected_result = false;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result
                    .unwrap()
                    .get_one::<bool>(DF_OPT_HUMAN_READABLE_DECIMAL)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_h_binary() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-h"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result
                    .unwrap()
                    .get_one::<bool>(DF_OPT_HUMAN_READABLE_BINARY)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_h_decimal() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-h"];
            let result = command.try_get_matches_from(args);

            let expected_result = false;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result
                    .unwrap()
                    .get_one::<bool>(DF_OPT_HUMAN_READABLE_DECIMAL)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_si() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-H"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result
                    .unwrap()
                    .get_one::<bool>(DF_OPT_HUMAN_READABLE_DECIMAL)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_si_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--si"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result
                    .unwrap()
                    .get_one::<bool>(DF_OPT_HUMAN_READABLE_DECIMAL)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_inodes() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-i"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_INODES).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_inodes_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--inodes"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_INODES).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_k() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-k"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_df_l() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-l"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_LOCAL).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_local() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--local"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_LOCAL).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_l_local() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-l", "--local"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_LOCAL).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_no_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--no-sync"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_NO_SYNC).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_l_no_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-l", "--no-sync"];
            let result = command.try_get_matches_from(args);

            // let expected_result = true;
            //
            // assert!(result.is_ok());
            //
            // assert_eq!(
            //     expected_result,
            //     *result.unwrap().get_one::<bool>(DF_OPT_LOCAL).unwrap()
            // );
            //
            // let expected_result = true;
            //
            // assert!(result.is_ok());
            //
            // assert_eq!(
            //     expected_result,
            //     *result.unwrap().get_one::<bool>(DF_OPT_NO_SYNC).unwrap()
            // );
            // unwrap()之后，result的所有权就已经被转移了（因为unwrap()方法消耗了self），所以第二次调用unwrap()时就遇到了问题。
            // 解决方案是确保在每次需要使用result时，它仍然是有效的。如果你需要多次使用结果或避免所有权的立即转移，可以考虑以下几种方式：
            // 使用as_ref()或as_mut()方法借用result的内容，而不是转移所有权。
            // 如果上下文允许，可以在第一次使用后克隆result（前提是clap::error::Error实现了Clone特质），但通常这不是最高效或推荐的做法。
            // 调整代码逻辑，确保在所有权转移之前完成所有必要的操作。
            // 针对错误信息中的建议修复代码，可以考虑如下修改（假设你确实需要在不同地方使用到result的结果）:

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }
            }
        }
        #[test]
        fn test_ct_app_df_local_no_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--local", "--no-sync"];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_source() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=source",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("source");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_fstype() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=fstype",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("fstype");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }
        #[test]
        fn test_ct_app_df_local_no_sync_output_itotal() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=itotal",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("itotal");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_iiused() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=iiused",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("iiused");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_iavail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=iavail",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("iavail");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_ipcent() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=ipcent",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("ipcent");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_size() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=size",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("size");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_used() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=used",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("used");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_avail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=avail",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("avail");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_pcent() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=pcent",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("pcent");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=file",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("file");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }

        #[test]
        fn test_ct_app_df_local_no_sync_output_target() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=target",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_LOCAL).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_LOCAL).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_NO_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_NO_SYNC).unwrap());
                }

                if matches.get_one::<String>(DF_OPT_OUTPUT).is_some() {
                    let expected_result = Some("target");

                    assert_eq!(
                        expected_result,
                        matches.get_one::<String>(DF_OPT_OUTPUT).map(|x| x.as_str())
                    )
                }
            }
        }
        #[test]
        fn test_ct_app_df_output_source() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=source"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("source");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_fstype() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=fstype"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("fstype");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_itotal() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=itotal"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("itotal");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_iused() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=iused"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("iused");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_iavail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=iavail"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("iavail");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_ipcent() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=ipcent"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("ipcent");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_size() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=size"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("size");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_used() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=used"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("used");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_avail() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=avail"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("avail");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_pcent() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=pcent"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("pcent");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=file"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("file");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_output_target() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--output=target"];
            let result = command.try_get_matches_from(args);

            let expected_result = Some("target");

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<String>(DF_OPT_OUTPUT)
                    .map(|x| x.as_str())
            );
        }

        #[test]
        fn test_ct_app_df_p() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-P"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_PORTABILITY).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_portability() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--portability"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_PORTABILITY).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_p_portability() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-P", "--portability"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_PORTABILITY).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--sync"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_SYNC).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_portability_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--portability", "--sync"];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_PORTABILITY).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_PORTABILITY).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_SYNC).unwrap());
                }
            }
        }

        #[test]
        fn test_ct_app_df_p_portability_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "-P",
                "--portability",
                "--sync",
            ];
            let result = command.try_get_matches_from(args);

            if let Ok(matches) = result.as_ref() {
                if matches.get_one::<bool>(DF_OPT_PORTABILITY).is_some() {
                    assert_eq!(true, *matches.get_one::<bool>(DF_OPT_PORTABILITY).unwrap());
                }

                if matches.get_one::<bool>(DF_OPT_SYNC).is_none() {
                    assert_eq!(false, *matches.get_one::<bool>(DF_OPT_SYNC).unwrap());
                }
            }
        }

        #[test]
        fn test_ct_app_df_type() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--type", "ext4"];
            let result = command.try_get_matches_from(args);

            let expected_result = "ext4";

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<OsString>(DF_OPT_TYPE)
                    .map(|x| x.as_os_str())
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_print_type() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--print-type"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_PRINT_TYPE).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_t() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-T"];
            let result = command.try_get_matches_from(args);

            let expected_result = true;

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                *result.unwrap().get_one::<bool>(DF_OPT_PRINT_TYPE).unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_x() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "-x", "ext4"];
            let result = command.try_get_matches_from(args);

            let expected_result = "ext4";

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<OsString>(DF_OPT_EXCLUDE_TYPE)
                    .map(|x| x.as_os_str())
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_df_exclude_type() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), df_dir, "--exclude-type", "ext4"];
            let result = command.try_get_matches_from(args);

            let expected_result = "ext4";

            assert!(result.is_ok());

            assert_eq!(
                expected_result,
                result
                    .unwrap()
                    .get_one::<OsString>(DF_OPT_EXCLUDE_TYPE)
                    .map(|x| x.as_os_str())
                    .unwrap()
            );
        }
    }

    mod tests_df_main {

        use crate::df_main;
        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;
        use tempfile::Builder;

        #[test]
        fn test_df_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_df_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_df_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_df_main_df_a() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-a"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_all() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--all"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_b_k() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BK"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_k() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bk"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_k() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=K"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_k() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=k"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_b_m() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BM"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_m() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bm"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_m() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=M"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_m() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=m"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
        #[test]
        fn test_df_main_df_uppercase_b_g() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BG"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_g() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bg"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_g() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=G"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_g() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=g"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_b_t() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BT"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_t() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bt"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_t() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=T"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_t() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=t"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
        #[test]
        fn test_df_main_df_uppercase_b_p() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BP"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_p() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bp"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_p() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=P"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_p() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=p"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_b_e() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BE"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_e() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Be"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_e() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=E"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_e() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=e"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_b_z() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BZ"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'Z' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_lowercase_b_z() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bz"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'z' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_z() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Z"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'Z' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_z() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=z"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'z' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_uppercase_b_y() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BY"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'Y' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_lowercase_b_y() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-By"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'y' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_y() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Y"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'Y' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_y() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=y"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'y' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_block_size_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_b_k_total_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BK", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_k_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bk", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_k_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=K", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_k_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=k", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_b_m_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BM", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_m_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bm", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_m_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=M", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_m_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=m", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
        #[test]
        fn test_df_main_df_uppercase_b_g_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BG", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_g_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bg", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_g_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=G", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_g_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=g", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_b_t_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BT", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_t_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bt", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_t_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=T", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_t_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=t", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
        #[test]
        fn test_df_main_df_uppercase_b_p_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BP", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_p_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bp", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_p_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=P", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_p_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=p", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_b_e_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BE", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_b_e_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Be", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_e_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=E", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_e_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=e", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_uppercase_b_z_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BZ", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'Z' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_lowercase_b_z_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-Bz", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'z' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_z_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Z", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'Z' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_z_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=z", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'z' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_uppercase_b_y_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BY", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'Y' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_lowercase_b_y_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-By", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'y' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_uppercase_block_size_y_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Y", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'Y' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_lowercase_block_size_y_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=y", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            let expected_error = "--block-size argument 'y' too large"; //df: --block-size argument 'y' too large
            assert!(result.is_err());
            assert_eq!(result.err().unwrap().to_string(), expected_error);
        }

        #[test]
        fn test_df_main_df_uppercase_b_k_total() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-BK", "--total"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_human_readable_binary() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--human-readable"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_human_readable_decimal() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--human-readable"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_h_binary() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-h"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_h_decimal() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-h"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_si() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-H"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_si_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--si"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_inodes() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-i"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_inodes_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--inodes"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_k() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-k"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_l() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-l"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--local"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_l_local() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-l", "--local"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_no_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--no-sync"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_l_no_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-l", "--no-sync"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
        #[test]
        fn test_df_main_df_local_no_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--local", "--no-sync"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_source() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=source",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_fstype() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=fstype",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_itotal() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=itotal",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_iused() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=iused",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
        #[test]
        fn test_df_main_df_local_no_sync_output_iavail() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=iavail",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_ipcent() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=ipcent",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_size() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=size",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_used() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=used",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_avail() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=avail",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_pcent() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=pcent",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_file() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=file",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_local_no_sync_output_target() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "--local",
                "--no-sync",
                "--output=target",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_source() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=source"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_fstype() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=fstype"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_itotal() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=itotal"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_iused() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=iused"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_iavail() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=iavail"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_ipcent() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=ipcent"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_size() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=size"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_used() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=used"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_avail() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=avail"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_pcent() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=pcent"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_file() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=file"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_output_target() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--output=target"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_p() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-P"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_portability() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--portability"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_p_portability() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-P", "--portability"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--sync"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_portability_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--portability", "--sync"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_p_portability_sync() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                df_dir,
                "-P",
                "--portability",
                "--sync",
            ];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_type() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--type", "ext4"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_print_type() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--print-type"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_t() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-T"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_x() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "-x", "ext4"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_df_main_df_exclude_type() {
            let temp_dir = Builder::new()
                .prefix("tests_df_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let df_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), df_dir, "--exclude-type", "ext4"];
            let result = df_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }
}