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

// spell-checker:ignore (ToDO) rwxr sourcepath targetpath Isnt uioerror

mod mode;

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::ct_backup_control::{self, CtBackupMode};
use ctcore::ct_display::Quotable;
use ctcore::ct_entries::{grp2gid, usr2uid};
use ctcore::ct_error::{CTError, CTIoError, CTResult, FromIo};
use ctcore::ct_fs::dir_strip_dot_for_creation;
use ctcore::ct_mode::get_umask;
use ctcore::ct_perms::{CtVerbosityLevel, Verbosity, wrap_chown};
use ctcore::ct_process::{getegid, geteuid};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show, ct_show_error, uio_error};
use file_diff::diff;
use filetime::{FileTime, set_file_times};
use std::error::Error;
use std::fmt::{Debug, Display};
use std::fs;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::path::{Path, PathBuf};
use std::process;

const DEFAULT_MODE: u32 = 0o755;
const DEFAULT_STRIP_PROGRAM: &str = "strip";

#[allow(dead_code)]
pub struct Installer {
    main_function: MainFunction,
    specified_mode: Option<u32>,
    backup_mode: CtBackupMode,
    suffix: String,
    owner_id: Option<u32>,
    group_id: Option<u32>,
    verbose: bool,
    preserve_timestamps: bool,
    compare: bool,
    strip: bool,
    strip_program: String,
    create_leading: bool,
    target_dir: Option<String>,
}

impl Default for Installer {
    fn default() -> Self {
        Self {
            main_function: MainFunction::Standard,
            specified_mode: None,
            backup_mode: CtBackupMode::NoBackup,
            suffix: String::new(),
            owner_id: None,
            group_id: None,
            verbose: false,
            preserve_timestamps: false,
            compare: false,
            strip: false,
            strip_program: String::from(DEFAULT_STRIP_PROGRAM),
            create_leading: false,
            target_dir: None,
        }
    }
}

#[derive(Debug)]
enum InstallError {
    Unimplemented(String),
    DirNeedsArg(),
    ChmodFailed(PathBuf),
    ChownFailed(PathBuf, String),
    InvalidTarget(PathBuf),
    TargetDirIsntDir(PathBuf),
    BackupFailed(PathBuf, PathBuf, std::io::Error),
    InstallFailed(PathBuf, PathBuf, std::io::Error),
    StripProgramFailed(String),
    MetadataFailed(std::io::Error),
    InvalidUser(String),
    InvalidGroup(String),
    CreateDirFailed(PathBuf, std::io::Error),
    OmittingDirectory(PathBuf),
    NotADirectory(PathBuf),
}

impl CTError for InstallError {
    fn code(&self) -> i32 {
        match self {
            Self::Unimplemented(_) => 2,
            _ => 1,
        }
    }

    fn usage(&self) -> bool {
        false
    }
}

impl Error for InstallError {}

impl Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unimplemented(opt) => write!(f, "Unimplemented feature: {opt}"),
            Self::DirNeedsArg() => {
                write!(
                    f,
                    "{} with -d requires at least one argument.",
                    ctcore::ct_util_name()
                )
            }
            Self::CreateDirFailed(dir, e) => {
                Display::fmt(&uio_error!(e, "failed to create {}", dir.quote()), f)
            }
            Self::ChmodFailed(file) => write!(f, "failed to chmod {}", file.quote()),
            Self::ChownFailed(file, msg) => write!(f, "failed to chown {}: {}", file.quote(), msg),
            Self::InvalidTarget(target) => write!(
                f,
                "invalid target {}: No such file or directory",
                target.quote()
            ),
            Self::TargetDirIsntDir(target) => {
                write!(f, "target {} is not a directory", target.quote())
            }
            Self::BackupFailed(from, to, e) => Display::fmt(
                &uio_error!(e, "cannot backup {} to {}", from.quote(), to.quote()),
                f,
            ),
            Self::InstallFailed(from, to, e) => Display::fmt(
                &uio_error!(e, "cannot install {} to {}", from.quote(), to.quote()),
                f,
            ),
            Self::StripProgramFailed(msg) => write!(f, "strip program failed: {msg}"),
            Self::MetadataFailed(e) => Display::fmt(&uio_error!(e, ""), f),
            Self::InvalidUser(user) => write!(f, "invalid user: {}", user.quote()),
            Self::InvalidGroup(group) => write!(f, "invalid group: {}", group.quote()),
            Self::OmittingDirectory(dir) => write!(f, "omitting directory {}", dir.quote()),
            Self::NotADirectory(dir) => {
                write!(f, "failed to access {}: Not a directory", dir.quote())
            }
        }
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum MainFunction {
    /// Create directories
    Directory,
    /// Install files to locations (primary functionality)
    Standard,
}

impl Installer {
    /// Determine the mode for chmod after copy.
    pub fn mode(&self) -> u32 {
        match self.specified_mode {
            Some(x) => x,
            None => DEFAULT_MODE,
        }
    }

    fn parse_main_function(matches: &ArgMatches) -> (MainFunction, bool) {
        let main_function = if matches.get_flag(install_options::INSTALL_DIRECTORY) {
            MainFunction::Directory
        } else {
            MainFunction::Standard
        };
        let considering_dir = MainFunction::Directory == main_function;
        (main_function, considering_dir)
    }

    fn parse_mode(matches: &ArgMatches, considering_dir: bool) -> CTResult<Option<u32>> {
        if matches.contains_id(install_options::INSTALL_MODE) {
            let x = matches
                .get_one::<String>(install_options::INSTALL_MODE)
                .ok_or(1)?;
            Ok(Some(
                mode::install_parse(x, considering_dir, get_umask()).map_err(|err| {
                    ct_show_error!("Invalid mode string: {}", err);
                    1
                })?,
            ))
        } else {
            Ok(None)
        }
    }

    fn check_conflicts(preserve_timestamps: bool, compare: bool, strip: bool) -> CTResult<()> {
        if preserve_timestamps && compare {
            ct_show_error!("Options --compare and --preserve-timestamps are mutually exclusive");
            return Err(1.into());
        }
        if compare && strip {
            ct_show_error!("Options --compare and --strip are mutually exclusive");
            return Err(1.into());
        }
        Ok(())
    }

    fn parse_owner(matches: &ArgMatches) -> CTResult<Option<u32>> {
        let owner = matches
            .get_one::<String>(install_options::INSTALL_OWNER)
            .map(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        if owner.is_empty() {
            Ok(None)
        } else {
            match usr2uid(&owner) {
                Ok(u) => Ok(Some(u)),
                Err(_) => Err(InstallError::InvalidUser(owner).into()),
            }
        }
    }

    fn parse_group(matches: &ArgMatches) -> CTResult<Option<u32>> {
        let group = matches
            .get_one::<String>(install_options::INSTALL_GROUP)
            .map(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        if group.is_empty() {
            Ok(None)
        } else {
            match grp2gid(&group) {
                Ok(g) => Ok(Some(g)),
                Err(_) => Err(InstallError::InvalidGroup(group).into()),
            }
        }
    }

    pub fn new(matches: &ArgMatches) -> CTResult<Self> {
        let (main_function, considering_dir) = Self::parse_main_function(matches);
        let specified_mode = Self::parse_mode(matches, considering_dir)?;
        let backup_mode = ct_backup_control::determine_backup_mode(matches)?;
        let target_dir = matches
            .get_one::<String>(install_options::INSTALL_TARGET_DIRECTORY)
            .cloned();

        let preserve_timestamps = matches.get_flag(install_options::INSTALL_PRESERVE_TIMESTAMPS);
        let compare = matches.get_flag(install_options::INSTALL_COMPARE);
        let strip = matches.get_flag(install_options::INSTALL_STRIP);

        Self::check_conflicts(preserve_timestamps, compare, strip)?;

        let owner_id = Self::parse_owner(matches)?;
        let group_id = Self::parse_group(matches)?;

        Ok(Self {
            main_function,
            specified_mode,
            backup_mode,
            suffix: ct_backup_control::determine_backup_suffix(matches),
            owner_id,
            group_id,
            verbose: matches.get_flag(install_options::INSTALL_VERBOSE),
            preserve_timestamps,
            compare,
            strip,
            strip_program: String::from(
                matches
                    .get_one::<String>(install_options::INSTALL_STRIP_PROGRAM)
                    .map(|s| s.as_str())
                    .unwrap_or(DEFAULT_STRIP_PROGRAM),
            ),
            create_leading: matches.get_flag(install_options::INSTALL_CREATE_LEADING),
            target_dir,
        })
    }

    fn validate_target_dir(&self, target_dir: &Path) -> CTResult<()> {
        if !target_dir.exists() {
            return Err(InstallError::InvalidTarget(target_dir.to_path_buf()).into());
        }
        if !target_dir.is_dir() {
            return Err(InstallError::TargetDirIsntDir(target_dir.to_path_buf()).into());
        }
        Ok(())
    }

    fn process_source_file(&self, from: &Path, to: &Path) -> CTResult<()> {
        if from.is_dir() {
            return Err(InstallError::OmittingDirectory(from.to_path_buf()).into());
        }

        if self.create_leading {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| InstallError::CreateDirFailed(parent.to_path_buf(), e))?;

                // 验证创建的目录
                if !parent.is_dir() {
                    return Err(InstallError::NotADirectory(parent.to_path_buf()).into());
                }
            }
        }

        copy(from, to, self)
    }

    /// 检查文件元数据。
    ///
    /// # 参数
    /// * `from` - 源文件路径
    /// * `to` - 目标文件路径
    ///
    /// # 返回值
    /// 返回 `CTResult<Option<(fs::Metadata, fs::Metadata)>>`，包含源文件和目标文件的元数据
    fn check_metadata(
        &self,
        from: &Path,
        to: &Path,
    ) -> CTResult<Option<(fs::Metadata, fs::Metadata)>> {
        let from_meta = match fs::metadata(from) {
            Ok(meta) => meta,
            Err(_) => return Ok(None),
        };

        let to_meta = match fs::metadata(to) {
            Ok(meta) => meta,
            Err(_) => return Ok(None),
        };

        Ok(Some((from_meta, to_meta)))
    }

    /// 检查文件的特殊权限位。
    ///
    /// # 参数
    /// * `from_meta` - 源文件元数据
    /// * `to_meta` - 目标文件元数据
    ///
    /// # 返回值
    /// 如果存在特殊权限位返回 true
    fn check_special_modes(&self, from_meta: &fs::Metadata, to_meta: &fs::Metadata) -> bool {
        let extra_mode: u32 = 0o7000; // setuid | setgid | sticky

        self.specified_mode.unwrap_or(0) & extra_mode != 0
            || from_meta.mode() & extra_mode != 0
            || to_meta.mode() & extra_mode != 0
    }

    /// 检查文件的基本属性。
    ///
    /// # 参数
    /// * `from_meta` - 源文件元数据
    /// * `to_meta` - 目标文件元数据
    ///
    /// # 返回值
    /// 如果属性不匹配返回 true
    fn check_file_attributes(&self, from_meta: &fs::Metadata, to_meta: &fs::Metadata) -> bool {
        let all_modes: u32 = 0o7777;

        // 检查基本属性
        if self.mode() != to_meta.mode() & all_modes
            || !from_meta.is_file()
            || !to_meta.is_file()
            || from_meta.len() != to_meta.len()
        {
            return true;
        }

        false
    }

    /// 检查文件的所有权。
    ///
    /// # 参数
    /// * `to_meta` - 目标文件元数据
    ///
    /// # 返回值
    /// 如果所有权不匹配返回 true
    fn check_ownership(&self, to_meta: &fs::Metadata) -> bool {
        // 检查所有者
        if let Some(owner_id) = self.owner_id {
            if owner_id != to_meta.uid() {
                return true;
            }
        }

        // 检查组
        if let Some(group_id) = self.group_id {
            if group_id != to_meta.gid() {
                return true;
            }
        } else {
            #[cfg(not(target_os = "windows"))]
            if to_meta.uid() != geteuid() || to_meta.gid() != getegid() {
                return true;
            }
        }

        false
    }

    fn need_copy(&self, from: &Path, to: &Path) -> CTResult<bool> {
        // 获取并检查元数据
        let (from_meta, to_meta) = match self.check_metadata(from, to)? {
            Some(meta) => meta,
            None => return Ok(true),
        };

        // 检查特殊权限位
        if self.check_special_modes(&from_meta, &to_meta) {
            return Ok(true);
        }

        // 检查文件属性
        if self.check_file_attributes(&from_meta, &to_meta) {
            return Ok(true);
        }

        // 检查所有权
        if self.check_ownership(&to_meta) {
            return Ok(true);
        }

        // 比较文件内容
        if !diff(from.to_str().unwrap(), to.to_str().unwrap()) {
            return Ok(true);
        }

        Ok(false)
    }

    /// 创建目录。
    ///
    /// # 参数
    /// * `path` - 要创建的目录路径
    ///
    /// # 返回值
    /// 返回 `CTResult<()>`，表示操作是否成功
    fn create_directory(&self, path: &Path) -> CTResult<()> {
        // 如果路径已存在，不需要创建
        if path.exists() {
            return Ok(());
        }

        // 处理特殊情况：install -d foo/. 应该创建 foo/
        let path_to_create = dir_strip_dot_for_creation(path);

        // 创建目录及其所有父目录
        fs::create_dir_all(path_to_create.as_path())
            .map_err_context(|| path_to_create.as_path().maybe_quote().to_string())?;

        if self.verbose {
            println!("creating directory {}", path_to_create.quote());
        }

        Ok(())
    }

    /// 设置目录权限模式。
    ///
    /// # 参数
    /// * `path` - 目录路径
    ///
    /// # 返回值
    /// 返回 `CTResult<()>`，表示操作是否成功
    fn set_directory_mode(&self, path: &Path) -> CTResult<()> {
        if mode::install_chmod(path, self.mode()).is_err() {
            ctcore::ct_error::set_ct_exit_code(1);
            return Err(InstallError::ChmodFailed(path.to_path_buf()).into());
        }
        Ok(())
    }

    fn get_chown_ids(&self) -> Option<(u32, u32)> {
        if self.owner_id.is_some() || self.group_id.is_some() {
            Some((
                self.owner_id.unwrap_or(geteuid()),
                self.group_id.unwrap_or(getegid()),
            ))
        } else if geteuid() == 0 {
            // 特殊情况：root 用户
            Some((0, 0))
        } else {
            None
        }
    }

    fn get_verbosity(&self) -> Verbosity {
        Verbosity {
            groups_only: self.owner_id.is_none(),
            level: CtVerbosityLevel::Normal,
        }
    }
}

const INSTALL_ABOUT: &str = ct_help_about!("install.md");
const INSTALL_USAGE: &str = ct_help_usage!("install.md");

mod install_options {
    pub const INSTALL_COMPARE: &str = "compare";
    pub const INSTALL_DIRECTORY: &str = "directory";
    pub const INSTALL_IGNORED: &str = "ignored";
    pub const INSTALL_CREATE_LEADING: &str = "create-leading";
    pub const INSTALL_GROUP: &str = "group";
    pub const INSTALL_MODE: &str = "mode";
    pub const INSTALL_OWNER: &str = "owner";
    pub const INSTALL_PRESERVE_TIMESTAMPS: &str = "preserve-timestamps";
    pub const INSTALL_STRIP: &str = "strip";
    pub const INSTALL_STRIP_PROGRAM: &str = "strip-program";
    pub const INSTALL_TARGET_DIRECTORY: &str = "target-directory";
    pub const INSTALL_NO_TARGET_DIRECTORY: &str = "no-target-directory";
    pub const INSTALL_VERBOSE: &str = "verbose";
    pub const INSTALL_PRESERVE_CONTEXT: &str = "preserve-context";
    pub const INSTALL_CONTEXT: &str = "context";

    pub const INSTALL_FILES: &str = "files";
}
/// Main install utility function, called from main.rs.
///
/// Returns a program return code.
///
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    install_main(args)
}

pub fn install_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let paths: Vec<String> = matches
        .get_many::<String>(install_options::INSTALL_FILES)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    check_unimplemented(&matches)?;

    let install = Installer::new(&matches)?;

    match install.main_function {
        MainFunction::Directory => install_directory(&paths, &install),
        MainFunction::Standard => install_standard(paths, &install),
    }
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(install_options::INSTALL_IGNORED)
            .short('c')
            .help("ignored")
            .action(ArgAction::SetTrue),
        Arg::new(install_options::INSTALL_COMPARE)
            .short('C')
            .long(install_options::INSTALL_COMPARE)
            .help(
                "compare each pair of source and destination files, and in some cases, \
                do not modify the destination at all",
            )
            .action(ArgAction::SetTrue),
        Arg::new(install_options::INSTALL_DIRECTORY)
            .short('d')
            .long(install_options::INSTALL_DIRECTORY)
            .help(
                "treat all arguments as directory names. create all components of \
                    the specified directories",
            )
            .action(ArgAction::SetTrue),
        // TODO implement flag
        Arg::new(install_options::INSTALL_CREATE_LEADING)
            .short('D')
            .help(
                "create all leading components of DEST except the last, then copy \
                    SOURCE to DEST",
            )
            .action(ArgAction::SetTrue),
        Arg::new(install_options::INSTALL_GROUP)
            .short('g')
            .long(install_options::INSTALL_GROUP)
            .help("set group ownership, instead of process's current group")
            .value_name("GROUP"),
        Arg::new(install_options::INSTALL_MODE)
            .short('m')
            .long(install_options::INSTALL_MODE)
            .help("set permission mode (as in chmod), instead of rwxr-xr-x")
            .value_name("MODE"),
        Arg::new(install_options::INSTALL_OWNER)
            .short('o')
            .long(install_options::INSTALL_OWNER)
            .help("set ownership (super-user only)")
            .value_name("OWNER")
            .value_hint(clap::ValueHint::Username),
        Arg::new(install_options::INSTALL_PRESERVE_TIMESTAMPS)
            .short('p')
            .long(install_options::INSTALL_PRESERVE_TIMESTAMPS)
            .help(
                "apply access/modification times of SOURCE files to \
                corresponding destination files",
            )
            .action(ArgAction::SetTrue),
        Arg::new(install_options::INSTALL_STRIP)
            .short('s')
            .long(install_options::INSTALL_STRIP)
            .help("strip symbol tables (no action Windows)")
            .action(ArgAction::SetTrue),
        Arg::new(install_options::INSTALL_STRIP_PROGRAM)
            .long(install_options::INSTALL_STRIP_PROGRAM)
            .help("program used to strip binaries (no action Windows)")
            .value_name("PROGRAM")
            .value_hint(clap::ValueHint::CommandName),
        // TODO implement flag
        Arg::new(install_options::INSTALL_TARGET_DIRECTORY)
            .short('t')
            .long(install_options::INSTALL_TARGET_DIRECTORY)
            .help("move all SOURCE arguments into DIRECTORY")
            .value_name("DIRECTORY")
            .value_hint(clap::ValueHint::DirPath),
        // TODO implement flag
        Arg::new(install_options::INSTALL_NO_TARGET_DIRECTORY)
            .short('T')
            .long(install_options::INSTALL_NO_TARGET_DIRECTORY)
            .help("(unimplemented) treat DEST as a normal file")
            .action(ArgAction::SetTrue),
        Arg::new(install_options::INSTALL_VERBOSE)
            .short('v')
            .long(install_options::INSTALL_VERBOSE)
            .help("explain what is being done")
            .action(ArgAction::SetTrue),
        // TODO implement flag
        Arg::new(install_options::INSTALL_PRESERVE_CONTEXT)
            .short('P')
            .long(install_options::INSTALL_PRESERVE_CONTEXT)
            .help("(unimplemented) preserve security context")
            .action(ArgAction::SetTrue),
        // TODO implement flag
        Arg::new(install_options::INSTALL_CONTEXT)
            .short('Z')
            .long(install_options::INSTALL_CONTEXT)
            .help("(unimplemented) set security context of files and directories")
            .value_name("CONTEXT")
            .action(ArgAction::SetTrue),
        Arg::new(install_options::INSTALL_FILES)
            .action(ArgAction::Append)
            .num_args(1..)
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(INSTALL_ABOUT)
        .override_usage(ct_format_usage(INSTALL_USAGE))
        .infer_long_args(true)
        .arg(ct_backup_control::arguments::backup())
        .arg(ct_backup_control::arguments::backup_no_args())
        .arg(ct_backup_control::arguments::suffix())
        .args(args)
}

/// Check for unimplemented command line arguments.
///
/// Either return the degenerate Ok value, or an Err with string.
///
/// # Errors
///
/// Error datum is a string of the unimplemented argument.
///
///
fn check_unimplemented(matches: &ArgMatches) -> CTResult<()> {
    if matches.get_flag(install_options::INSTALL_NO_TARGET_DIRECTORY) {
        Err(InstallError::Unimplemented(String::from("--no-target-directory, -T")).into())
    } else if matches.get_flag(install_options::INSTALL_PRESERVE_CONTEXT) {
        Err(InstallError::Unimplemented(String::from("--preserve-context, -P")).into())
    } else if matches.get_flag(install_options::INSTALL_CONTEXT) {
        Err(InstallError::Unimplemented(String::from("--context, -Z")).into())
    } else {
        Ok(())
    }
}

/// 创建目录并设置其权限。
///
/// # 功能
/// - 创建指定的目录及其所有父目录
/// - 设置目录的权限模式
/// - 支持创建多个目录
/// - 处理特殊路径（如 foo/.）
///
/// # 参数
/// * `paths` - 要创建的目录路径列表
/// * `b` - 安装器配置，包含权限模式和其他选项
///
/// # 错误处理
/// - 如果路径列表为空，返回 `DirNeedsArg` 错误
/// - 目录创建失败时继续处理其他目录
/// - 权限设置失败时继续处理其他目录
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功完成
fn install_directory(paths: &[String], b: &Installer) -> CTResult<()> {
    if paths.is_empty() {
        return Err(InstallError::DirNeedsArg().into());
    }

    for path in paths.iter().map(Path::new) {
        if let Err(e) = b.create_directory(path) {
            ct_show!(e);
            continue;
        }

        if let Err(e) = b.set_directory_mode(path) {
            ct_show!(e);
            continue;
        }
    }

    Ok(())
}

#[cfg(not(unix))]
fn is_potential_directory_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.ends_with(MAIN_SEPARATOR) || path_str.ends_with('/') || path.is_dir()
}

/// 安装文件到指定位置。
///
/// # 功能
/// - 将源文件复制到目标位置
/// - 设置文件权限和所有权
/// - 支持备份已存在的文件
/// - 可选择保留时间戳
///
/// # 参数
/// * `paths` - 源文件路径列表，最后一个路径为目标位置
/// * `b` - 安装器配置
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功完成
fn install_standard(paths: Vec<String>, b: &Installer) -> CTResult<()> {
    // 验证路径数量
    if paths.len() < 2 && b.target_dir.is_none() {
        return Ok(());
    }

    // 获取并验证目标目录
    let target_dir = if let Some(ref dir) = b.target_dir {
        PathBuf::from(dir)
    } else {
        PathBuf::from(&paths[paths.len() - 1])
    };
    b.validate_target_dir(&target_dir)?;

    // 处理源文件
    let sources = if b.target_dir.is_some() {
        &paths[..]
    } else {
        &paths[..paths.len() - 1]
    };

    for from in sources.iter().map(Path::new) {
        let file_name = match from.file_name() {
            Some(name) => name,
            None => continue,
        };
        let to = target_dir.join(file_name);

        if let Err(e) = b.process_source_file(from, &to) {
            ct_show!(e);
            ctcore::ct_error::set_ct_exit_code(1);
        }
    }

    Ok(())
}

/// 处理文件的所有者和组设置。
///
/// # 功能
/// - 根据配置设置文件的所有者和组
/// - 处理 root 用户的特殊情况
/// - 支持只设置组而不设置所有者
///
/// # 参数
/// * `path` - 要设置所有权的文件路径
/// * `b` - 安装器配置，包含所有者和组的设置
///
/// # 特殊情况
/// - 如果用户是 root (uid=0)，且未指定所有者和组，则设置为 root:root
/// - 如果只指定了组，则只修改组所有权
/// - 如果既未指定所有者也未指定组，则不进行任何修改
///
/// # 错误处理
/// - 如果所有者或组无效，返回相应错误
/// - 如果 chown 操作失败，返回错误
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功完成
fn chown_optional_user_group(path: &Path, b: &Installer) -> CTResult<()> {
    // 获取所有者和组 ID
    let Some((owner_id, group_id)) = b.get_chown_ids() else {
        return Ok(());
    };

    // 执行 chown 操作
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) => return Err(InstallError::MetadataFailed(e).into()),
    };

    let verbosity = b.get_verbosity();
    match wrap_chown(
        path,
        &meta,
        Some(owner_id),
        Some(group_id),
        false,
        verbosity,
    ) {
        Ok(msg) if b.verbose && !msg.is_empty() => println!("chown: {msg}"),
        Ok(_) => {}
        Err(e) => return Err(InstallError::ChownFailed(path.to_path_buf(), e).into()),
    }

    Ok(())
}

/// 在覆盖文件前执行备份操作。
///
/// # 功能
/// - 检查目标文件是否存在
/// - 根据备份模式创建备份文件
/// - 支持自定义备份后缀
/// - 可选择显示备份操作的详细信息
///
/// # 参数
/// * `to` - 要备份的目标文件路径
/// * `b` - 安装器配置，包含备份模式和后缀设置
///
/// # 错误处理
/// - 如果备份操作失败，返回 `BackupFailed` 错误
/// - 如果目标文件不存在，返回 `None`
///
/// # 返回值
/// 返回 `CTResult<Option<PathBuf>>`
/// - `Some(PathBuf)` - 备份文件的路径
/// - `None` - 不需要备份（目标文件不存在）
fn perform_backup(to: &Path, b: &Installer) -> CTResult<Option<PathBuf>> {
    if to.exists() {
        if b.verbose {
            println!("removed {}", to.quote());
        }
        let backup_path = ct_backup_control::get_backup_path(b.backup_mode, to, &b.suffix);
        if let Some(ref backup_path) = backup_path {
            // TODO!!
            if let Err(err) = fs::rename(to, backup_path) {
                return Err(
                    InstallError::BackupFailed(to.to_path_buf(), backup_path.clone(), err).into(),
                );
            }
        }
        Ok(backup_path)
    } else {
        Ok(None)
    }
}

/// 将文件从源路径复制到目标路径。
///
/// # 功能
/// - 处理目标文件已存在的情况
/// - 支持复制 /dev/null
/// - 自动删除目标位置的无效符号链接
///
/// # 参数
/// * `from` - 源文件路径
/// * `to` - 目标文件路径
///
/// # 错误处理
/// - 如果删除现有文件失败，返回错误
/// - 如果复制操作失败，返回 `InstallFailed` 错误
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功完成
fn copy_file(from: &Path, to: &Path) -> CTResult<()> {
    // fs::copy fails if destination is a invalid symlink.
    // so lets just remove all existing files at destination before copy.
    if let Err(e) = fs::remove_file(to) {
        if e.kind() != std::io::ErrorKind::NotFound {
            ct_show_error!(
                "Failed to remove existing file {}. Error: {:?}",
                to.display(),
                e
            );
        }
    }

    if from.as_os_str() == "/dev/null" {
        /* workaround a limitation of fs::copy
         * https://github.com/rust-lang/rust/issues/79390
         */
        if let Err(err) = File::create(to) {
            return Err(
                InstallError::InstallFailed(from.to_path_buf(), to.to_path_buf(), err).into(),
            );
        }
    } else if let Err(err) = fs::copy(from, to) {
        return Err(InstallError::InstallFailed(from.to_path_buf(), to.to_path_buf(), err).into());
    }
    Ok(())
}

/// 使用外部程序对文件进行 strip 操作。
///
/// # 功能
/// - 使用配置的 strip 程序处理文件
/// - 处理以连字符开头的文件名
/// - 在 strip 失败时删除目标文件
///
/// # 参数
/// * `to` - 要处理的文件路径
/// * `b` - 安装器配置，包含 strip 程序设置
///
/// # 错误处理
/// - 如果 strip 程序执行失败，返回 `StripProgramFailed` 错误
/// - 在失败时自动清理目标文件
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功完成
fn strip_file(to: &Path, b: &Installer) -> CTResult<()> {
    let target_path = prepare_strip_path(to);
    execute_strip(&target_path, &b.strip_program)
}

fn prepare_strip_path(path: &Path) -> PathBuf {
    // 处理以连字符开头的文件名
    if path
        .as_os_str()
        .to_str()
        .unwrap_or_default()
        .starts_with('-')
    {
        let mut new_path = PathBuf::from(".");
        new_path.push(path);
        new_path
    } else {
        path.to_path_buf()
    }
}

fn execute_strip(path: &Path, strip_program: &str) -> CTResult<()> {
    match process::Command::new(strip_program).arg(path).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => {
            // 如果 strip 失败，删除目标文件
            let _ = fs::remove_file(path);
            Err(InstallError::StripProgramFailed(format!(
                "strip process terminated abnormally - exit code: {}",
                status.code().unwrap()
            ))
            .into())
        }
        Err(e) => {
            // 如果 strip 失败，删除目标文件
            let _ = fs::remove_file(path);
            Err(InstallError::StripProgramFailed(e.to_string()).into())
        }
    }
}

/// 设置文件的所有权和权限。
///
/// # 功能
/// - 设置文件权限模式
/// - 设置文件所有者和组
/// - 支持自定义权限模式
///
/// # 参数
/// * `to` - 要设置属性的文件路径
/// * `b` - 安装器配置，包含权限和所有权设置
///
/// # 错误处理
/// - 如果 chmod 操作失败，返回 `ChmodFailed` 错误
/// - 如果 chown 操作失败，返回相应错误
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功完成
fn set_ownership_and_permissions(to: &Path, b: &Installer) -> CTResult<()> {
    // Silent the warning as we want to the error message
    #[allow(clippy::question_mark)]
    if mode::install_chmod(to, b.mode()).is_err() {
        return Err(InstallError::ChmodFailed(to.to_path_buf()).into());
    }

    chown_optional_user_group(to, b)?;

    Ok(())
}

/// 保留文件的时间戳。
///
/// # 功能
/// - 复制源文件的访问时间和修改时间到目标文件
/// - 保持文件元数据的一致性
///
/// # 参数
/// * `from` - 源文件路径
/// * `to` - 目标文件路径
///
/// # 错误处理
/// - 如果获取源文件元数据失败，返回 `MetadataFailed` 错误
/// - 如果设置时间戳失败，显示错误但继续执行
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功完成
fn preserve_timestamps(from: &Path, to: &Path) -> CTResult<()> {
    let meta = match fs::metadata(from) {
        Ok(meta) => meta,
        Err(e) => return Err(InstallError::MetadataFailed(e).into()),
    };

    let modified_time = FileTime::from_last_modification_time(&meta);
    let accessed_time = FileTime::from_last_access_time(&meta);

    match set_file_times(to, accessed_time, modified_time) {
        Ok(_) => Ok(()),
        Err(e) => {
            ct_show_error!("{}", e);
            Ok(())
        }
    }
}

/// 复制文件并设置其属性。
///
/// # 功能
/// - 复制文件内容
/// - 设置权限和所有权
/// - 可选择保留时间戳
/// - 支持文件备份
///
/// # 参数
/// * `from` - 源文件路径
/// * `to` - 目标文件路径
/// * `b` - 安装器配置
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功完成
fn copy(from: &Path, to: &Path, b: &Installer) -> CTResult<()> {
    if b.compare && !b.need_copy(from, to)? {
        return Ok(());
    }
    // Declare the path here as we may need it for the verbose output below.
    let backup_path = perform_backup(to, b)?;

    copy_file(from, to)?;

    #[cfg(not(windows))]
    if b.strip {
        strip_file(to, b)?;
    }

    set_ownership_and_permissions(to, b)?;

    if b.preserve_timestamps {
        preserve_timestamps(from, to)?;
    }

    if b.verbose {
        print!("{} -> {}", from.quote(), to.quote());
        match backup_path {
            Some(path) => println!(" (backup: {})", path.quote()),
            None => println!(),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn test_need_copy() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");

        // 创建源文件
        File::create(&source).unwrap();

        // 测试目标文件不存在的情况
        let installer = Installer {
            main_function: MainFunction::Standard,
            specified_mode: None,
            backup_mode: CtBackupMode::NoBackup,
            suffix: String::new(),
            owner_id: None,
            group_id: None,
            verbose: false,
            preserve_timestamps: false,
            compare: false,
            strip: false,
            strip_program: String::from(DEFAULT_STRIP_PROGRAM),
            create_leading: false,
            target_dir: None,
        };
        assert!(installer.need_copy(&source, &dest).unwrap());

        // 创建目标文件并测试
        File::create(&dest).unwrap();
        assert!(installer.need_copy(&source, &dest).unwrap());
    }

    #[test]
    fn test_perform_backup() {
        let temp = tempdir().unwrap();
        let dest = temp.path().join("dest");

        // 创建目标文件
        File::create(&dest).unwrap();

        let installer = Installer {
            backup_mode: CtBackupMode::SimpleBackup,
            suffix: String::from("~"),
            verbose: true,
            ..Default::default()
        };

        let backup = perform_backup(&dest, &installer).unwrap();
        assert!(backup.is_some());
        assert!(backup.unwrap().ends_with("dest~"));
    }

    #[test]
    fn test_copy_file() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");

        // 创建源文件
        std::fs::write(&source, "test content").unwrap();

        // 测试复制
        copy_file(&source, &dest).unwrap();

        assert!(dest.exists());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "test content");
    }

    #[test]
    fn test_set_ownership_and_permissions() {
        let temp = tempdir().unwrap();
        let file = temp.path().join("test");

        // 创建测试文件
        File::create(&file).unwrap();

        let installer = Installer {
            specified_mode: Some(0o644),
            owner_id: None,
            group_id: None,
            ..Default::default()
        };

        set_ownership_and_permissions(&file, &installer).unwrap();

        let metadata = std::fs::metadata(&file).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o644);
    }

    #[test]
    fn test_preserve_timestamps() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");

        // 创建源文件和目标文件
        File::create(&source).unwrap();
        File::create(&dest).unwrap();

        preserve_timestamps(&source, &dest).unwrap();

        let source_meta = std::fs::metadata(&source).unwrap();
        let dest_meta = std::fs::metadata(&dest).unwrap();

        assert_eq!(
            source_meta.modified().unwrap(),
            dest_meta.modified().unwrap()
        );
    }
}
