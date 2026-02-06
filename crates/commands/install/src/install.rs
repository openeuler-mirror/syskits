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

extern crate rust_i18n; // spell-checker:ignore (ToDO) rwxr sourcepath targetpath Isnt uioerror
/// install 命令的实现 - 复制文件并设置属性
///
/// 此模块实现了 install 命令的功能,用于复制文件并设置其属性。
/// 主要功能包括:
/// - 复制文件到目标位置
/// - 创建目录
/// - 设置文件权限和所有权
/// - 支持备份已存在的文件
/// - 支持保留时间戳
/// - 支持 strip 二进制文件
///
/// # 主要结构体
/// - `Installer`: 存储 install 命令的配置和执行方法
/// - `MainFunction`: 定义主要操作模式
///
/// # 主要函数
/// - `install_main()`: 命令入口函数
/// - `install_directory()`: 创建目录
/// - `install_standard()`: 安装文件
/// - `copy()`: 复制文件并设置属性
mod mode;

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_backup_control::{self, CtBackupMode};
use ctcore::ct_display::Quotable;
use ctcore::ct_entries::{grp2gid, usr2uid};
use ctcore::ct_error::{CTError, CTIoError, CTResult, FromIo};
use ctcore::ct_fs::dir_strip_dot_for_creation;
use ctcore::ct_mode::get_umask;
use ctcore::ct_perms::{CtVerbosityLevel, Verbosity, wrap_chown};
use ctcore::ct_process::{getegid, geteuid};
use ctcore::{ct_show, ct_show_error, uio_error};
use file_diff::diff;
use filetime::{FileTime, set_file_times};
#[cfg(target_os = "linux")]
use selinux::{self, SecurityContext};
use std::error::Error;
use std::ffi::{CString, OsStr, OsString};
use std::fmt::{Debug, Display};
use std::fs;
use std::fs::File;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::path::{Path, PathBuf};
use std::process;
use sys_locale::get_locale;

const DEFAULT_MODE: u32 = 0o755;
const DEFAULT_STRIP_PROGRAM: &str = "strip";

/// install 命令的配置和执行器
pub struct Installer {
    /// 主要操作模式(目录创建或文件安装)
    main_function: MainFunction,
    /// 指定的权限模式
    specified_mode: Option<u32>,
    /// 备份模式
    backup_mode: CtBackupMode,
    /// 备份文件后缀
    suffix: String,
    /// 所有者ID
    owner_id: Option<u32>,
    /// 组ID
    group_id: Option<u32>,
    /// 是否显示详细信息
    verbose: bool,
    /// 是否保留时间戳
    preserve_timestamps: bool,
    /// 是否比较文件内容
    compare: bool,
    /// 是否执行strip操作
    strip: bool,
    /// strip程序路径
    strip_program: String,
    /// 是否创建父目录
    create_leading: bool,
    /// 指定的目标目录
    target_dir: Option<String>,
    /// 将目标视为普通文件
    no_target_dir: bool,
    /// 是否保留安全上下文
    preserve_context: bool,
    /// 是否设置安全上下文（-Z/--context）
    set_context: bool,
    /// 指定的安全上下文
    context: Option<OsString>,
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
            no_target_dir: false,
            preserve_context: false,
            set_context: false,
            context: None,
        }
    }
}

/// install 命令的命令行选项定义
mod install_options {
    /// 在复制前比较源文件和目标文件
    pub const INSTALL_COMPARE: &str = "compare";

    /// 创建目录而不是复制文件
    pub const INSTALL_DIRECTORY: &str = "directory";

    /// 忽略的选项(未使用)
    pub const INSTALL_IGNORED: &str = "ignored";

    /// 创建所有必要的父目录
    pub const INSTALL_CREATE_LEADING: &str = "create-leading";

    /// 设置目标文件的用户组
    pub const INSTALL_GROUP: &str = "group";

    /// 设置目标文件的权限模式
    pub const INSTALL_MODE: &str = "mode";

    /// 设置目标文件的所有者
    pub const INSTALL_OWNER: &str = "owner";

    /// 保留源文件的时间戳
    pub const INSTALL_PRESERVE_TIMESTAMPS: &str = "preserve-timestamps";

    /// 对二进制文件执行strip操作
    pub const INSTALL_STRIP: &str = "strip";

    /// 指定strip程序的路径
    pub const INSTALL_STRIP_PROGRAM: &str = "strip-program";

    /// 指定目标目录
    pub const INSTALL_TARGET_DIRECTORY: &str = "target-directory";

    /// 将目标视为普通文件而不是目录
    pub const INSTALL_NO_TARGET_DIRECTORY: &str = "no-target-directory";

    /// 显示详细操作信息
    pub const INSTALL_VERBOSE: &str = "verbose";

    /// 保留文件的安全上下文
    pub const INSTALL_PRESERVE_CONTEXT: &str = "preserve-context";

    /// 设置文件的安全上下文
    pub const INSTALL_CONTEXT: &str = "context";

    /// 要处理的文件列表
    pub const INSTALL_FILES: &str = "files";
}

/// 安装命令可能遇到的错误类型
#[derive(Debug)]
enum InstallError {
    /// chmod 操作失败
    /// 参数: 目标文件路径
    ChmodFailed(PathBuf),

    /// chown 操作失败
    /// 参数:
    /// - 目标文件路径
    /// - 错误信息
    ChownFailed(PathBuf, String),

    /// 目标路径无效(不存在)
    /// 参数: 无效的目标路径
    InvalidTarget(PathBuf),

    /// 备份文件失败
    /// 参数:
    /// - 源文件路径
    /// - 目标备份文件路径
    /// - IO错误信息
    BackupFailed(PathBuf, PathBuf, std::io::Error),

    /// 安装(复制)文件失败
    /// 参数:
    /// - 源文件路径
    /// - 目标文件路径
    /// - IO错误信息
    InstallFailed(PathBuf, PathBuf, std::io::Error),

    /// strip 程序执行失败
    /// 参数: 错误信息
    StripProgramFailed(String),

    /// 获取文件元数据失败
    /// 参数: IO错误信息
    MetadataFailed(std::io::Error),

    /// 指定的用户不存在
    /// 参数: 用户名
    InvalidUser(String),

    /// 指定的用户组不存在
    /// 参数: 组名
    InvalidGroup(String),

    /// 创建目录失败
    /// 参数:
    /// - 目录路径
    /// - IO错误信息
    CreateDirFailed(PathBuf, std::io::Error),

    /// 跳过目录(当不应处理目录时)
    /// 参数: 目录路径
    OmittingDirectory(PathBuf),

    /// 路径不是目录
    /// 参数: 路径
    NotADirectory(PathBuf),
}

impl CTError for InstallError {
    fn code(&self) -> i32 {
        1
    }

    fn usage(&self) -> bool {
        false
    }
}

impl Error for InstallError {}

impl Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // 创建目录失败,显示具体错误原因
            Self::CreateDirFailed(dir, e) => {
                Display::fmt(&uio_error!(e, "failed to create {}", dir.quote()), f)
            }

            // 修改文件权限失败
            Self::ChmodFailed(file) => write!(f, "failed to chmod {}", file.quote()),

            // 修改文件所有者失败,包含具体错误信息
            Self::ChownFailed(file, msg) => write!(f, "failed to chown {}: {}", file.quote(), msg),

            // 目标路径不存在
            Self::InvalidTarget(target) => write!(
                f,
                "failed to access {}: No such file or directory",
                target.quote()
            ),

            // 备份文件失败,显示源文件、目标文件和错误原因
            Self::BackupFailed(from, to, e) => Display::fmt(
                &uio_error!(e, "cannot backup {} to {}", from.quote(), to.quote()),
                f,
            ),

            // 安装文件失败,显示源文件、目标文件和错误原因
            Self::InstallFailed(from, to, e) => Display::fmt(
                &uio_error!(e, "cannot install {} to {}", from.quote(), to.quote()),
                f,
            ),

            // strip 程序执行失败
            Self::StripProgramFailed(msg) => write!(f, "strip program failed: {msg}"),

            // 获取文件元数据失败
            Self::MetadataFailed(e) => Display::fmt(&uio_error!(e, ""), f),

            // 指定的用户不存在
            Self::InvalidUser(user) => write!(f, "invalid user: {}", user.quote()),

            // 指定的用户组不存在
            Self::InvalidGroup(group) => write!(f, "invalid group: {}", group.quote()),

            // 跳过目录
            Self::OmittingDirectory(dir) => write!(f, "omitting directory {}", dir.quote()),

            // 目标路径不是目录
            Self::NotADirectory(dir) => {
                write!(f, "failed to access {}: Not a directory", dir.quote())
            }
        }
    }
}

/// 主要操作模式
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum MainFunction {
    /// 创建目录
    Directory,
    /// 安装文件(主要功能)
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
        // 检查时间戳保留和比较选项的冲突
        if preserve_timestamps && compare {
            ct_show_error!(
                "options --compare (-C) and --preserve-timestamps are mutually exclusive"
            );
            return Err(1.into());
        }

        // 检查比较和strip选项的冲突
        if compare && strip {
            ct_show_error!("options --compare (-C) and --strip are mutually exclusive");
            return Err(1.into());
        }
        Ok(())
    }

    fn parse_owner(matches: &ArgMatches) -> CTResult<Option<u32>> {
        // 获取所有者名称
        let owner = matches
            .get_one::<String>(install_options::INSTALL_OWNER)
            .map(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        // 如果未指定所有者,返回None
        if owner.is_empty() {
            Ok(None)
        } else {
            // 将所有者名称转换为ID
            match usr2uid(&owner) {
                Ok(u) => Ok(Some(u)),
                Err(_) => Err(InstallError::InvalidUser(owner).into()),
            }
        }
    }

    fn parse_group(matches: &ArgMatches) -> CTResult<Option<u32>> {
        // 获取组名称
        let group = matches
            .get_one::<String>(install_options::INSTALL_GROUP)
            .map(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        // 如果未指定组,返回None
        if group.is_empty() {
            Ok(None)
        } else {
            // 将组名称转换为ID
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
        let mut compare = matches.get_flag(install_options::INSTALL_COMPARE);
        let strip = matches.get_flag(install_options::INSTALL_STRIP);
        let no_target_dir = matches.get_flag(install_options::INSTALL_NO_TARGET_DIRECTORY);
        let preserve_context = matches.get_flag(install_options::INSTALL_PRESERVE_CONTEXT);
        let context = matches
            .get_one::<OsString>(install_options::INSTALL_CONTEXT)
            .cloned();
        let set_context = matches.contains_id(install_options::INSTALL_CONTEXT);

        Self::check_conflicts(preserve_timestamps, compare, strip)?;

        let owner_id = Self::parse_owner(matches)?;
        let group_id = Self::parse_group(matches)?;

        if main_function == MainFunction::Directory && strip {
            ct_show_error!("the strip option may not be used when installing a directory");
            return Err(1.into());
        }
        if main_function == MainFunction::Directory && target_dir.is_some() {
            ct_show_error!("target directory not allowed when installing a directory");
            return Err(1.into());
        }
        if no_target_dir && target_dir.is_some() {
            ct_show_error!("cannot combine --target-directory (-t) and --no-target-directory (-T)");
            return Err(1.into());
        }

        if compare {
            if let Some(mode) = specified_mode {
                if mode & !0o777 != 0 {
                    ct_show_error!(
                        "the --compare (-C) option is ignored when you specify a mode with non-permission bits"
                    );
                    compare = false;
                }
            }
        }

        let strip_program = matches
            .get_one::<String>(install_options::INSTALL_STRIP_PROGRAM)
            .map(|s| s.as_str())
            .unwrap_or(DEFAULT_STRIP_PROGRAM);
        if !strip && matches.contains_id(install_options::INSTALL_STRIP_PROGRAM) {
            ct_show_error!(
                "WARNING: ignoring --strip-program option as -s option was not specified"
            );
        }

        let (preserve_context, set_context, context) =
            normalize_context_flags(preserve_context, set_context, context)?;

        if preserve_context && set_context {
            ct_show_error!("cannot set target context and preserve it");
            return Err(1.into());
        }

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
            strip_program: String::from(strip_program),
            create_leading: matches.get_flag(install_options::INSTALL_CREATE_LEADING),
            target_dir,
            no_target_dir,
            preserve_context,
            set_context,
            context,
        })
    }

    fn validate_target_dir(&self, target_dir: &Path) -> CTResult<()> {
        if !target_dir.exists() {
            return Err(InstallError::InvalidTarget(target_dir.to_path_buf()).into());
        }
        if !target_dir.is_dir() {
            return Err(InstallError::NotADirectory(target_dir.to_path_buf()).into());
        }
        Ok(())
    }

    fn process_source_file(&self, from: &Path, to: &Path) -> CTResult<()> {
        if from.is_dir() {
            return Err(InstallError::OmittingDirectory(from.to_path_buf()).into());
        }

        if self.create_leading {
            if let Some(parent) = to.parent() {
                create_leading_dirs(parent, self.verbose)
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
        let from_lmeta = match fs::symlink_metadata(from) {
            Ok(meta) => meta,
            Err(_) => return Ok(true),
        };
        let to_lmeta = match fs::symlink_metadata(to) {
            Ok(meta) => meta,
            Err(_) => return Ok(true),
        };
        if !from_lmeta.file_type().is_file() || !to_lmeta.file_type().is_file() {
            return Ok(true);
        }

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

        #[cfg(target_os = "linux")]
        if self.preserve_context && selinux_supported() {
            let from_ctx = match SecurityContext::of_path(from, true, false) {
                Ok(Some(ctx)) => ctx,
                _ => return Ok(true),
            };
            let to_ctx = match SecurityContext::of_path(to, true, false) {
                Ok(Some(ctx)) => ctx,
                _ => return Ok(true),
            };
            if from_ctx.as_bytes() != to_ctx.as_bytes() {
                return Ok(true);
            }
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

        // 创建目录及其所有父目录，并在 verbose 模式下打印每一层
        create_leading_dirs(path_to_create.as_path(), self.verbose)
            .map_err_context(|| path_to_create.as_path().maybe_quote().to_string())?;

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

pub fn install_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
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
            .help(t!("install.clap.install_ignored"))
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
            .help(t!("install.clap.install_group"))
            .value_name("GROUP"),
        Arg::new(install_options::INSTALL_MODE)
            .short('m')
            .long(install_options::INSTALL_MODE)
            .help(t!("install.clap.install_mode"))
            .value_name("MODE"),
        Arg::new(install_options::INSTALL_OWNER)
            .short('o')
            .long(install_options::INSTALL_OWNER)
            .help(t!("install.clap.install_owner"))
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
            .help(t!("install.clap.install_strip"))
            .action(ArgAction::SetTrue),
        Arg::new(install_options::INSTALL_STRIP_PROGRAM)
            .long(install_options::INSTALL_STRIP_PROGRAM)
            .help(t!("install.clap.install_strip_program"))
            .value_name("PROGRAM")
            .value_hint(clap::ValueHint::CommandName),
        // TODO implement flag
        Arg::new(install_options::INSTALL_TARGET_DIRECTORY)
            .short('t')
            .long(install_options::INSTALL_TARGET_DIRECTORY)
            .help(t!("install.clap.install_target_directory"))
            .value_name("DIRECTORY")
            .value_hint(clap::ValueHint::DirPath),
        // TODO implement flag
        Arg::new(install_options::INSTALL_NO_TARGET_DIRECTORY)
            .short('T')
            .long(install_options::INSTALL_NO_TARGET_DIRECTORY)
            .help(t!("install.clap.install_no_target_directory"))
            .action(ArgAction::SetTrue),
        Arg::new(install_options::INSTALL_VERBOSE)
            .short('v')
            .long(install_options::INSTALL_VERBOSE)
            .help(t!("install.clap.install_verbose"))
            .action(ArgAction::SetTrue),
        // TODO implement flag
        Arg::new(install_options::INSTALL_PRESERVE_CONTEXT)
            .long(install_options::INSTALL_PRESERVE_CONTEXT)
            .help(t!("install.clap.install_preserve_context"))
            .action(ArgAction::SetTrue),
        // TODO implement flag
        Arg::new(install_options::INSTALL_CONTEXT)
            .short('Z')
            .long(install_options::INSTALL_CONTEXT)
            .help(t!("install.clap.install_context"))
            .value_name("CONTEXT")
            .num_args(0..=1)
            .value_parser(clap::builder::ValueParser::os_string()),
        Arg::new(install_options::INSTALL_FILES)
            .action(ArgAction::Append)
            .num_args(1..)
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("install.about"))
        .override_usage(t!("install.usage"))
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
fn check_unimplemented(_matches: &ArgMatches) -> CTResult<()> {
    Ok(())
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
/// - 如果路径列表为空，返回错误
/// - 目录创建失败时继续处理其他目录
/// - 权限设置失败时继续处理其他目录
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功完成
fn install_directory(paths: &[String], b: &Installer) -> CTResult<()> {
    if paths.is_empty() {
        ct_show_error!("missing file operand");
        return Err(1.into());
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
    if paths.is_empty() {
        ct_show_error!("missing file operand");
        return Err(1.into());
    }
    if b.no_target_dir && paths.len() > 2 {
        ct_show_error!("extra operand {}", paths[2].as_str().quote());
        return Err(1.into());
    }
    if b.target_dir.is_none() && paths.len() < 2 {
        ct_show_error!(
            "missing destination file operand after {}",
            paths[0].as_str().quote()
        );
        return Err(1.into());
    }

    // 处理两种模式:
    // 1. install source dest (单文件复制)
    // 2. install source1 source2 ... target_dir (多文件复制到目录)
    if b.target_dir.is_none() && paths.len() == 2 {
        // 单文件复制模式: install source dest
        let from = Path::new(&paths[0]);
        let to = Path::new(&paths[1]);

        if b.no_target_dir {
            if to.is_dir() {
                ct_show_error!("cannot create regular file {}: Is a directory", to.quote());
                ctcore::ct_error::set_ct_exit_code(1);
                return Err(1.into());
            }
            if let Err(e) = b.process_source_file(from, to) {
                ct_show!(e);
                ctcore::ct_error::set_ct_exit_code(1);
            }
            return Ok(());
        }

        // 如果目标是已存在的目录,则复制到该目录内
        if to.is_dir() {
            if from.is_dir() {
                ct_show!(InstallError::OmittingDirectory(from.to_path_buf()));
                ctcore::ct_error::set_ct_exit_code(1);
                return Ok(());
            }
            let file_name = match from.file_name() {
                Some(name) => name,
                None => {
                    ct_show!(InstallError::OmittingDirectory(from.to_path_buf()));
                    ctcore::ct_error::set_ct_exit_code(1);
                    return Ok(());
                }
            };
            let to = to.join(file_name);
            if let Err(e) = b.process_source_file(from, &to) {
                ct_show!(e);
                ctcore::ct_error::set_ct_exit_code(1);
            }
        } else {
            // 否则直接复制为新文件
            if let Err(e) = b.process_source_file(from, to) {
                ct_show!(e);
                ctcore::ct_error::set_ct_exit_code(1);
            }
        }
        return Ok(());
    }

    // 多文件复制模式: 目标必须是目录
    let target_dir = if let Some(ref dir) = b.target_dir {
        PathBuf::from(dir)
    } else {
        PathBuf::from(&paths[paths.len() - 1])
    };
    if !target_dir.exists() {
        if b.create_leading {
            create_leading_dirs(&target_dir, b.verbose)
                .map_err(|e| InstallError::CreateDirFailed(target_dir.clone(), e))?;
        } else {
            return Err(InstallError::InvalidTarget(target_dir).into());
        }
    }
    b.validate_target_dir(&target_dir)?;

    // 处理源文件
    let sources = if b.target_dir.is_some() {
        &paths[..]
    } else {
        &paths[..paths.len() - 1]
    };

    for from in sources.iter().map(Path::new) {
        if from.is_dir() {
            ct_show!(InstallError::OmittingDirectory(from.to_path_buf()));
            ctcore::ct_error::set_ct_exit_code(1);
            continue;
        }
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

#[cfg(target_os = "linux")]
fn selinux_supported() -> bool {
    selinux::kernel_support() != selinux::KernelSupport::Unsupported
}

#[cfg(not(target_os = "linux"))]
fn selinux_supported() -> bool {
    false
}

fn os_str_to_c_string(os_str: &OsStr) -> CString {
    CString::new(os_str.as_bytes()).expect("Failed to convert OsStr to CString")
}

fn normalize_context_flags(
    preserve_context: bool,
    set_context: bool,
    context: Option<OsString>,
) -> CTResult<(bool, bool, Option<OsString>)> {
    if !selinux_supported() {
        if preserve_context {
            eprintln!(
                "install: WARNING: ignoring --preserve-context; this kernel is not SELinux-enabled"
            );
        }
        if context.is_some() {
            eprintln!(
                "install: warning: ignoring --context; it requires an SELinux-enabled kernel"
            );
        }
        return Ok((false, false, None));
    }
    Ok((preserve_context, set_context, context))
}

#[cfg(target_os = "linux")]
fn apply_default_selinux_context(path: &Path, mode: u32) -> CTResult<()> {
    let _ = mode;
    SecurityContext::set_default_for_path(path).map_err(|e| {
        ct_show_error!(
            "warning: failed to restore context for {}: {e}",
            path.display()
        );
        1
    })?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_explicit_selinux_context(path: &Path, context: &OsString) -> CTResult<()> {
    let c_context = os_str_to_c_string(context);
    SecurityContext::from_c_str(&c_context, false)
        .set_for_path(path, true, false)
        .map_err(|e| {
            ct_show_error!("failed to set default file creation context: {e}");
            1
        })?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_preserve_context(from: &Path, to: &Path) -> CTResult<()> {
    let ctx = match SecurityContext::of_path(from, true, false) {
        Ok(Some(c)) => c,
        Ok(None) => return Ok(()),
        Err(e) => {
            ct_show_error!(
                "warning: failed to get security context for {}: {e}",
                from.display()
            );
            return Ok(());
        }
    };
    if let Err(e) = ctx.set_for_path(to, true, false) {
        ct_show_error!(
            "warning: failed to set security context for {}: {e}",
            to.display()
        );
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn apply_default_selinux_context(_path: &Path, _mode: u32) -> CTResult<()> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn apply_explicit_selinux_context(_path: &Path, _context: &OsString) -> CTResult<()> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn apply_preserve_context(_from: &Path, _to: &Path) -> CTResult<()> {
    Ok(())
}

fn create_leading_dirs(path: &Path, verbose: bool) -> Result<(), std::io::Error> {
    let mut cur = PathBuf::new();
    for component in path.components() {
        cur.push(component);
        if cur.exists() {
            if !cur.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "Not a directory",
                ));
            }
            continue;
        }
        fs::create_dir(&cur)?;
        if verbose {
            println!("install: creating directory {}", cur.quote());
        }
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

    if b.set_context {
        if let Some(ref ctx) = b.context {
            apply_explicit_selinux_context(to, ctx)?;
        } else {
            apply_default_selinux_context(to, b.mode())?;
        }
    }

    if b.preserve_context {
        apply_preserve_context(from, to)?;
    }

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

#[derive(Default)]
pub struct Install;
impl Tool for Install {
    fn name(&self) -> &'static str {
        "install"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        install_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs::File;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn test_tool_implementation() {
        let tool = Install;

        // Test name method
        assert_eq!(tool.name(), "install");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("install"));

        // Test execute method - help command should work
        let args: Vec<OsString> = vec![OsString::from("install"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

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
            no_target_dir: false,
            preserve_context: false,
            set_context: false,
            context: None,
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

    #[test]
    fn test_installer_new() {
        // 创建基本的命令行参数
        let mut cmd = Command::new("test");
        cmd = cmd
            .arg(ct_backup_control::arguments::backup())
            .arg(ct_backup_control::arguments::backup_no_args())
            .arg(ct_backup_control::arguments::suffix())
            .arg(
                Arg::new(install_options::INSTALL_TARGET_DIRECTORY)
                    .short('t')
                    .long(install_options::INSTALL_TARGET_DIRECTORY)
                    .value_name("DIRECTORY"),
            )
            .arg(
                Arg::new(install_options::INSTALL_FILES)
                    .value_name("FILES")
                    .num_args(1..)
                    .required(false),
            )
            .arg(
                Arg::new(install_options::INSTALL_COMPARE)
                    .short('C')
                    .long(install_options::INSTALL_COMPARE)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_PRESERVE_TIMESTAMPS)
                    .short('p')
                    .long(install_options::INSTALL_PRESERVE_TIMESTAMPS)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_STRIP)
                    .short('s')
                    .long(install_options::INSTALL_STRIP)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_STRIP_PROGRAM)
                    .long(install_options::INSTALL_STRIP_PROGRAM)
                    .value_name("PROGRAM"),
            )
            .arg(
                Arg::new(install_options::INSTALL_VERBOSE)
                    .short('v')
                    .long(install_options::INSTALL_VERBOSE)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_MODE)
                    .short('m')
                    .long(install_options::INSTALL_MODE)
                    .value_name("MODE"),
            )
            .arg(
                Arg::new(install_options::INSTALL_OWNER)
                    .short('o')
                    .long(install_options::INSTALL_OWNER)
                    .value_name("OWNER"),
            )
            .arg(
                Arg::new(install_options::INSTALL_GROUP)
                    .short('g')
                    .long(install_options::INSTALL_GROUP)
                    .value_name("GROUP"),
            )
            .arg(
                Arg::new(install_options::INSTALL_DIRECTORY)
                    .short('d')
                    .long(install_options::INSTALL_DIRECTORY)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_CREATE_LEADING)
                    .short('D')
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_NO_TARGET_DIRECTORY)
                    .short('T')
                    .long(install_options::INSTALL_NO_TARGET_DIRECTORY)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_PRESERVE_CONTEXT)
                    .long(install_options::INSTALL_PRESERVE_CONTEXT)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_CONTEXT)
                    .short('Z')
                    .long(install_options::INSTALL_CONTEXT)
                    .num_args(0..=1)
                    .value_parser(clap::builder::ValueParser::os_string()),
            );

        // 测试目录模式
        let matches = cmd
            .clone()
            .try_get_matches_from(vec!["test", "-d"])
            .unwrap();
        let installer = Installer::new(&matches).unwrap();
        assert_eq!(installer.main_function, MainFunction::Directory);

        // 测试权限模式
        let matches = cmd
            .clone()
            .try_get_matches_from(vec!["test", "-m", "644"])
            .unwrap();
        let installer = Installer::new(&matches).unwrap();
        assert_eq!(installer.specified_mode, Some(0o644));

        // 测试互斥选项
        let mut cmd = Command::new("test");
        cmd = cmd
            .arg(ct_backup_control::arguments::backup())
            .arg(ct_backup_control::arguments::backup_no_args())
            .arg(ct_backup_control::arguments::suffix())
            .arg(
                Arg::new(install_options::INSTALL_TARGET_DIRECTORY)
                    .short('t')
                    .long(install_options::INSTALL_TARGET_DIRECTORY)
                    .value_name("DIRECTORY"),
            )
            .arg(
                Arg::new(install_options::INSTALL_FILES)
                    .value_name("FILES")
                    .num_args(1..)
                    .required(false),
            )
            .arg(
                Arg::new(install_options::INSTALL_COMPARE)
                    .short('C')
                    .long(install_options::INSTALL_COMPARE)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_PRESERVE_TIMESTAMPS)
                    .short('p')
                    .long(install_options::INSTALL_PRESERVE_TIMESTAMPS)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_STRIP)
                    .short('s')
                    .long(install_options::INSTALL_STRIP)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_STRIP_PROGRAM)
                    .long(install_options::INSTALL_STRIP_PROGRAM)
                    .value_name("PROGRAM"),
            )
            .arg(
                Arg::new(install_options::INSTALL_VERBOSE)
                    .short('v')
                    .long(install_options::INSTALL_VERBOSE)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_MODE)
                    .short('m')
                    .long(install_options::INSTALL_MODE)
                    .value_name("MODE"),
            )
            .arg(
                Arg::new(install_options::INSTALL_OWNER)
                    .short('o')
                    .long(install_options::INSTALL_OWNER)
                    .value_name("OWNER"),
            )
            .arg(
                Arg::new(install_options::INSTALL_GROUP)
                    .short('g')
                    .long(install_options::INSTALL_GROUP)
                    .value_name("GROUP"),
            )
            .arg(
                Arg::new(install_options::INSTALL_DIRECTORY)
                    .short('d')
                    .long(install_options::INSTALL_DIRECTORY)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_NO_TARGET_DIRECTORY)
                    .short('T')
                    .long(install_options::INSTALL_NO_TARGET_DIRECTORY)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_PRESERVE_CONTEXT)
                    .long(install_options::INSTALL_PRESERVE_CONTEXT)
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new(install_options::INSTALL_CONTEXT)
                    .short('Z')
                    .long(install_options::INSTALL_CONTEXT)
                    .num_args(0..=1)
                    .value_parser(clap::builder::ValueParser::os_string()),
            );

        let matches = cmd
            .try_get_matches_from(vec![
                "test", "-C", "-p", "-s", "-v", "-m", "644", "-o", "user", "-g", "group", "-d",
            ])
            .unwrap();
        assert!(Installer::new(&matches).is_err());

        // 测试默认值
        let installer = Installer::default();

        assert_eq!(installer.main_function, MainFunction::Standard);
        assert_eq!(installer.specified_mode, None);
        assert_eq!(installer.backup_mode, CtBackupMode::NoBackup);
        assert!(!installer.verbose);
        assert!(!installer.preserve_timestamps);
        assert!(!installer.compare);
        assert!(!installer.strip);
        assert_eq!(installer.strip_program, DEFAULT_STRIP_PROGRAM);
        assert!(!installer.create_leading);
        assert!(installer.target_dir.is_none());
        assert!(!installer.no_target_dir);
        assert!(!installer.preserve_context);
        assert!(!installer.set_context);
        assert!(installer.context.is_none());
    }

    #[test]
    fn test_standard() {
        let temp = tempdir().unwrap();

        // 创建源文件
        let source = temp.path().join("source.txt");
        std::fs::write(&source, "test content").unwrap();

        // 创建目标目录
        let target_dir = temp.path().join("target");
        std::fs::create_dir(&target_dir).unwrap();

        // 基本安装测试
        let installer = Installer {
            main_function: MainFunction::Standard,
            specified_mode: Some(0o644),
            backup_mode: CtBackupMode::NoBackup,
            suffix: String::new(),
            owner_id: None,
            group_id: None,
            verbose: true,
            preserve_timestamps: false,
            compare: false,
            strip: false,
            strip_program: String::from(DEFAULT_STRIP_PROGRAM),
            create_leading: false,
            target_dir: Some(target_dir.to_string_lossy().into_owned()),
            no_target_dir: false,
            preserve_context: false,
            set_context: false,
            context: None,
        };

        let paths = vec![source.to_string_lossy().into_owned()];
        assert!(install_standard(paths, &installer).is_ok());

        // 验证文件是否正确安装
        let installed_file = target_dir.join("source.txt");
        assert!(installed_file.exists());
        assert_eq!(
            std::fs::read_to_string(&installed_file).unwrap(),
            "test content"
        );
        assert_eq!(installed_file.metadata().unwrap().mode() & 0o777, 0o644);

        // 测试备份功能
        let installer = Installer {
            backup_mode: CtBackupMode::SimpleBackup,
            suffix: String::from("~"),
            ..installer
        };

        let paths = vec![source.to_string_lossy().into_owned()];
        assert!(install_standard(paths, &installer).is_ok());

        // 验证备份文件是否创建
        let backup_file = target_dir.join("source.txt~");
        assert!(backup_file.exists());

        // 测试多文件安装
        let source2 = temp.path().join("source2.txt");
        std::fs::write(&source2, "test content 2").unwrap();

        let paths = vec![
            source.to_string_lossy().into_owned(),
            source2.to_string_lossy().into_owned(),
        ];
        assert!(install_standard(paths, &installer).is_ok());

        // 验证多个文件是否都安装成功
        assert!(target_dir.join("source.txt").exists());
        assert!(target_dir.join("source2.txt").exists());
    }

    #[test]
    fn test_install_error_display() {
        // 测试 ChmodFailed
        let err = InstallError::ChmodFailed(PathBuf::from("/test/path"));
        assert_eq!(err.to_string(), "failed to chmod '/test/path'");

        // 测试 ChownFailed
        let err =
            InstallError::ChownFailed(PathBuf::from("/test/path"), "permission denied".to_string());
        assert_eq!(
            err.to_string(),
            "failed to chown '/test/path': permission denied"
        );

        // 测试 InvalidTarget
        let err = InstallError::InvalidTarget(PathBuf::from("/test/path"));
        assert_eq!(
            err.to_string(),
            "failed to access '/test/path': No such file or directory"
        );

        // 测试 InvalidUser
        let err = InstallError::InvalidUser("testuser".to_string());
        assert_eq!(err.to_string(), "invalid user: 'testuser'");

        // 测试 InvalidGroup
        let err = InstallError::InvalidGroup("testgroup".to_string());
        assert_eq!(err.to_string(), "invalid group: 'testgroup'");
    }

    #[test]
    fn test_chown_optional_user_group() {
        let temp = tempdir().unwrap();
        let test_file = temp.path().join("test_file");
        File::create(&test_file).unwrap();

        // 测试默认情况（不设置所有者和组）
        let installer = Installer {
            owner_id: None,
            group_id: None,
            ..Default::default()
        };
        assert!(chown_optional_user_group(&test_file, &installer).is_ok());

        // 测试只设置组
        let installer = Installer {
            owner_id: None,
            group_id: Some(getegid()),
            ..Default::default()
        };
        assert!(chown_optional_user_group(&test_file, &installer).is_ok());

        // 测试同时设置所有者和组
        let installer = Installer {
            owner_id: Some(geteuid()),
            group_id: Some(getegid()),
            ..Default::default()
        };
        assert!(chown_optional_user_group(&test_file, &installer).is_ok());

        // 测试文件不存在的情况
        let nonexistent = temp.path().join("nonexistent");
        let installer = Installer {
            owner_id: Some(geteuid()),
            group_id: Some(getegid()),
            ..Default::default()
        };
        assert!(chown_optional_user_group(&nonexistent, &installer).is_err());
    }

    #[test]
    #[cfg(not(windows))] // strip 命令在 Windows 上不可用
    fn test_strip_file() {
        let temp = tempdir().unwrap();

        // 创建一个简单的二进制文件
        let exec_file = temp.path().join("test_exec");
        let mut file = File::create(&exec_file).unwrap();
        // 写入一些机器码，确保文件可以被 strip
        file.write_all(&[
            0x7f, 0x45, 0x4c, 0x46, // ELF 魔数
            0x02, 0x01, 0x01, 0x00, // 其他 ELF 头部信息
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ])
        .unwrap();

        // 设置可执行权限
        std::fs::set_permissions(&exec_file, std::fs::Permissions::from_mode(0o755)).unwrap();

        // 测试正常情况
        let installer = Installer {
            strip_program: String::from(DEFAULT_STRIP_PROGRAM),
            ..Default::default()
        };

        // 如果 strip 命令不可用，跳过测试
        if !std::path::Path::new(DEFAULT_STRIP_PROGRAM).exists() {
            return;
        }

        assert!(strip_file(&exec_file, &installer).is_ok());
        assert!(exec_file.exists()); // 文件应该还存在

        // 测试以连字符开头的文件名
        let hyphen_file = temp.path().join("-test_exec");
        std::fs::copy(&exec_file, &hyphen_file).unwrap();
        assert!(strip_file(&hyphen_file, &installer).is_ok());
        assert!(hyphen_file.exists());

        // 测试无效的 strip 程序
        let installer = Installer {
            strip_program: String::from("nonexistent_strip"),
            ..Default::default()
        };
        let test_file = temp.path().join("test_fail");
        std::fs::copy(&exec_file, &test_file).unwrap();

        let result = strip_file(&test_file, &installer);
        assert!(result.is_err());
        assert!(!test_file.exists()); // 失败时文件应该被删除
    }
}
