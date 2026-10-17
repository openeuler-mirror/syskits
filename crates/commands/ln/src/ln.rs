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

// spell-checker:ignore (ToDO) srcpath targetpath EEXIST

extern crate rust_i18n;
/// ln 命令的实现 - 创建文件链接
///
/// 此模块实现了 ln 命令的功能,支持创建硬链接和符号链接。
/// 主要功能包括:
/// - 创建硬链接和符号链接
/// - 支持相对路径和绝对路径
/// - 支持备份已存在的目标文件
/// - 支持递归创建目录
/// - 支持设置链接权限和所有权
///
/// # 主要结构体
/// - `LnSettings`: 存储 ln 命令的配置选项
/// - `OverwriteMode`: 定义目标文件存在时的处理方式
///
/// # 主要函数
/// - `ln_main()`: 命令入口函数
/// - `ln_exec()`: 执行链接操作
/// - `ln_link()`: 创建单个链接
/// - `link_files_in_dir()`: 在目录中创建多个链接
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};

use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_backup_control::{self, CtBackupMode};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, FromIo};
use ctcore::ct_fs::make_path_relative_to;
use ctcore::ct_fs::{MissingHandling, ResolveMode, canonicalize};
use ctcore::ct_show_error;
use ctcore::libc;
use std::borrow::Cow;
use std::collections::HashSet;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::Display;
use std::fs;
use std::io::{BufRead, Write};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, symlink};
#[cfg(windows)]
use std::os::windows::fs::{symlink_dir, symlink_file};
use std::path::{Path, PathBuf};
use sys_locale::get_locale;

#[cfg(unix)]
unsafe extern "C" {
    fn rpmatch(response: *const libc::c_char) -> libc::c_int;
}

/// ln 命令的配置选项
pub struct LnSettings {
    /// 目标文件存在时的处理方式
    overwrite: OverwriteMode,
    /// 备份模式
    backup: CtBackupMode,
    /// 备份文件后缀
    suffix: OsString,
    /// 是否创建符号链接
    is_symbolic: bool,
    /// 是否使用相对路径
    is_relative: bool,
    /// 是否跟随符号链接
    is_logical: bool,
    /// 指定的目标目录
    target_dir: Option<PathBuf>,
    /// 是否将目标视为普通文件
    is_no_target_dir: bool,
    /// 是否不解引用符号链接
    is_no_dereference: bool,
    /// 是否显示详细信息
    is_verbose: bool,
    /// 是否允许创建目录硬链接
    is_directory: bool,
}

/// 目标文件存在时的处理方式
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverwriteMode {
    /// 不覆盖已存在的文件
    NoClobber,
    /// 交互式确认是否覆盖
    Interactive,
    /// 强制覆盖
    Force,
}

#[derive(Debug)]
enum LnError {
    /// 目标路径应该是目录但不是目录
    /// 参数: 目标路径
    TargetIsDirectory(PathBuf),
    /// 部分链接创建失败
    /// 具体错误信息已在其他地方处理
    SomeLinksFailed,
    /// 缺少目标文件路径参数
    /// 参数: 最后一个提供的参数
    MissingDestination(PathBuf),
    /// 提供了多余的操作数
    /// 参数: 多余的操作数
    ExtraOperand(OsString),
    ///非root用户权限
    NonRootPermission,
    /// 源文件和目标文件是同一个文件
    SameFile(PathBuf, PathBuf),
    /// 不允许对目录创建硬链接
    HardLinkToDirectory(PathBuf),
    /// 目标硬链接已存在
    HardLinkAlreadyExists(PathBuf),
    /// 不允许覆盖目录
    CannotOverwriteDirectory(PathBuf),
}

impl Display for LnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // 目标路径是目录时的错误信息
            Self::TargetIsDirectory(s) => write!(f, "target {} is not a directory", s.quote()),

            Self::HardLinkToDirectory(s) => {
                write!(f, "{}: hard link not allowed for directory", s.display())
            }

            Self::HardLinkAlreadyExists(s) => {
                write!(f, "failed to create hard link {}: File exists", s.quote())
            }

            Self::CannotOverwriteDirectory(s) => {
                write!(f, "{}: cannot overwrite directory", s.quote())
            }

            // 源文件和目标文件是同一个文件时的错误信息
            Self::SameFile(s, d) => {
                write!(f, "{} and {} are the same file", s.quote(), d.quote())
            }

            // 部分链接创建失败时返回空字符串(错误信息已在其他地方处理)
            Self::SomeLinksFailed => Ok(()),

            // 缺少目标文件路径时的错误信息
            Self::MissingDestination(s) => {
                write!(f, "missing destination file operand after {}", s.quote())
            }

            // 有多余的操作数时的错误信息,并提示查看帮助
            Self::ExtraOperand(s) => write!(
                f,
                "extra operand {}\nTry '{} --help' for more information.",
                s.quote(),
                ctcore::ct_execute_phrase()
            ),

            //使用-d，-F，--directory 参数时，非root用户报错信息
            Self::NonRootPermission => {
                write!(f, "non root premission")
            }
        }
    }
}

impl Error for LnError {}

impl CTError for LnError {
    fn code(&self) -> i32 {
        1
    }
}

mod lnoptions {
    /// 强制覆盖已存在的目标文件
    pub const LN_FORCE: &str = "force";
    /// 交互式确认是否覆盖目标文件
    pub const LN_INTERACTIVE: &str = "interactive";
    /// 不解引用符号链接
    pub const LN_NO_DEREFERENCE: &str = "no-dereference";
    /// 创建符号链接而不是硬链接
    pub const LN_SYMBOLIC: &str = "symbolic";
    /// 解析符号链接(跟随符号链接)
    pub const LN_LOGICAL: &str = "logical";
    /// 不解析符号链接(使用物理路径)
    pub const LN_PHYSICAL: &str = "physical";
    /// 指定目标目录
    pub const LN_TARGET_DIRECTORY: &str = "target-directory";
    /// 将目标视为普通文件而不是目录
    pub const LN_NO_TARGET_DIRECTORY: &str = "no-target-directory";
    /// 创建相对符号链接
    pub const LN_RELATIVE: &str = "relative";
    /// 显示详细操作信息
    pub const LN_VERBOSE: &str = "verbose";
    /// 允许超级用户创建目录硬链接
    pub const LN_DIRECTORY: &str = "directory";
}

static LN_ARG_FILES: &str = "files";

#[derive(Default)]
pub struct Ln;
impl Tool for Ln {
    fn name(&self) -> &'static str {
        "ln"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        ln_main(args.iter().cloned())
    }
}

pub fn ln_main(args: impl ctcore::Args) -> CTResult<()> {
    #[cfg(unix)]
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    }

    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let after_help = format!(
        "{}\n\n{}",
        t!("ln.after_help"),
        ct_backup_control::CT_BACKUP_CONTROL_LONG_HELP
    );

    let matches = ct_app().after_help(after_help).try_get_matches_from(args)?;

    /* the list of files */

    let paths: Vec<PathBuf> = matches
        .get_many::<PathBuf>(LN_ARG_FILES)
        .unwrap()
        .cloned()
        .collect();

    let settings = LnSettings::new(&matches)?;

    ln_exec(&paths[..], &settings)
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(lnoptions::LN_FORCE)
            .short('f')
            .long(lnoptions::LN_FORCE)
            .help(t!("ln.clap.ln_force"))
            .overrides_with(lnoptions::LN_FORCE)
            .overrides_with(lnoptions::LN_INTERACTIVE)
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_INTERACTIVE)
            .short('i')
            .long(lnoptions::LN_INTERACTIVE)
            .help(t!("ln.clap.ln_interactive"))
            .overrides_with(lnoptions::LN_INTERACTIVE)
            .overrides_with(lnoptions::LN_FORCE)
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_NO_DEREFERENCE)
            .short('n')
            .long(lnoptions::LN_NO_DEREFERENCE)
            .help(
                "treat LINK_NAME as a normal file if it is a \
                    symbolic link to a directory",
            )
            .overrides_with(lnoptions::LN_NO_DEREFERENCE)
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_LOGICAL)
            .short('L')
            .long(lnoptions::LN_LOGICAL)
            .help(t!("ln.clap.ln_logical"))
            .overrides_with(lnoptions::LN_PHYSICAL)
            .overrides_with(lnoptions::LN_LOGICAL)
            .action(ArgAction::SetTrue),
        // Not implemented yet
        Arg::new(lnoptions::LN_PHYSICAL)
            .short('P')
            .long(lnoptions::LN_PHYSICAL)
            .help(t!("ln.clap.ln_physical"))
            .overrides_with(lnoptions::LN_PHYSICAL)
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_SYMBOLIC)
            .short('s')
            .long(lnoptions::LN_SYMBOLIC)
            .help(t!("ln.clap.ln_symbolic"))
            // override added for https://github.com/ctutils/coreutils/issues/2359
            .overrides_with(lnoptions::LN_SYMBOLIC)
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_TARGET_DIRECTORY)
            .short('t')
            .long(lnoptions::LN_TARGET_DIRECTORY)
            .help(t!("ln.clap.ln_target_directory"))
            .value_name("DIRECTORY")
            .value_hint(clap::ValueHint::DirPath)
            .value_parser(clap::builder::ValueParser::path_buf())
            .conflicts_with(lnoptions::LN_NO_TARGET_DIRECTORY),
        Arg::new(lnoptions::LN_NO_TARGET_DIRECTORY)
            .short('T')
            .long(lnoptions::LN_NO_TARGET_DIRECTORY)
            .help(t!("ln.clap.ln_no_target_directory"))
            .overrides_with(lnoptions::LN_NO_TARGET_DIRECTORY)
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_RELATIVE)
            .short('r')
            .long(lnoptions::LN_RELATIVE)
            .help(t!("ln.clap.ln_relative"))
            .requires(lnoptions::LN_SYMBOLIC)
            .overrides_with(lnoptions::LN_RELATIVE)
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_VERBOSE)
            .short('v')
            .long(lnoptions::LN_VERBOSE)
            .help(t!("ln.clap.ln_verbose"))
            .overrides_with(lnoptions::LN_VERBOSE)
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_DIRECTORY)
            .short('d')
            .visible_short_alias('F')
            .long(lnoptions::LN_DIRECTORY)
            .help("allow the superuser to try to hard link directories")
            .overrides_with(lnoptions::LN_DIRECTORY)
            .action(ArgAction::SetTrue),
        Arg::new(LN_ARG_FILES)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath)
            .value_parser(clap::builder::ValueParser::path_buf())
            .required(true)
            .num_args(1..),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("ln.about"))
        .override_usage(t!("ln.usage"))
        .infer_long_args(true)
        .arg(
            ct_backup_control::arguments::backup()
                .overrides_with(ct_backup_control::arguments::OPT_BACKUP),
        )
        .arg(
            ct_backup_control::arguments::backup_no_args()
                .overrides_with(ct_backup_control::arguments::OPT_BACKUP_NO_ARG),
        )
        .arg(
            ct_backup_control::arguments::suffix()
                .overrides_with(ct_backup_control::arguments::OPT_SUFFIX)
                .value_parser(clap::builder::ValueParser::os_string()),
        )
        .args(args)
}

/// 执行链接操作。支持四种不同的链接形式。
///
/// # 形式
/// 1. 直接链接：`ln [OPTION]... [-T] TARGET LINK_NAME`
/// 2. 链接到当前目录：`ln [OPTION]... TARGET`
/// 3. 链接到目录：`ln [OPTION]... TARGET... DIRECTORY`
/// 4. 使用 -t 选项：`ln [OPTION]... -t DIRECTORY TARGET...`
///
/// # 参数
/// * `files` - 源文件和目标路径的列表
/// * `settings` - 链接操作的配置选项
///
/// # 返回值
/// 返回 `CTResult<()>`，表示操作是否成功
///
/// # 错误
/// - 如果缺少目标文件，返回 `MissingDestination` 错误
/// - 如果有多余的操作数，返回 `ExtraOperand` 错误
fn ln_exec(files: &[PathBuf], settings: &LnSettings) -> CTResult<()> {
    // 处理第四种形式：使用 -t 选项指定目标目录
    if let Some(ref target_dir) = settings.target_dir {
        return link_files_in_dir(files, target_dir, settings);
    }

    // 如果没有指定 -T 选项
    if !settings.is_no_target_dir {
        // 处理第二种形式：单文件链接到当前目录
        if files.len() == 1 {
            return link_files_in_dir(files, &PathBuf::from("."), settings);
        }

        // 获取最后一个文件
        let last_file = files.last().unwrap();

        // 处理第三种形式：多文件链接到目录
        if files.len() > 2 || ln_target_is_dir(last_file, settings) {
            let target_dir = last_file;
            let source_files = &files[0..files.len() - 1];
            return link_files_in_dir(source_files, target_dir, settings);
        }
    }

    // 处理第一种形式：直接链接
    match files.len() {
        1 => Err(LnError::MissingDestination(files[0].clone()).into()),
        2 => ln_link(&files[0], &files[1], settings),
        _ => Err(LnError::ExtraOperand(files[2].clone().into()).into()),
    }
}

/// 在目标目录中创建多个链接。
///
/// # 功能
/// - 验证目标路径是否为目录
/// - 处理多个源文件的链接创建
/// - 防止重复链接创建
/// - 支持符号链接和硬链接
///
/// # 参数
/// * `files` - 源文件路径列表
/// * `target_dir` - 目标目录路径
/// * `settings` - 链接操作的配置选项
///
/// # 返回值
/// 返回 `CTResult<()>`，表示所有操作是否成功
///
/// # 错误
/// - 如果目标不是目录，返回 `TargetIsDirectory` 错误
/// - 如果任何链接创建失败，返回 `SomeLinksFailed` 错误
#[allow(clippy::cognitive_complexity)]
fn link_files_in_dir(files: &[PathBuf], target_dir: &Path, settings: &LnSettings) -> CTResult<()> {
    // 验证目标是否为目录
    if !ln_target_is_dir(target_dir, settings) {
        return Err(LnError::TargetIsDirectory(target_dir.to_owned()).into());
    }

    let mut linked_destinations = HashSet::with_capacity(files.len());
    let mut all_successful = true;

    for srcpath in files {
        // 构建目标路径
        let targetpath = build_target_path(srcpath, target_dir, settings)?;

        // 检查是否已创建该目标
        if linked_destinations.contains(&targetpath) {
            ct_show_error!(
                "will not overwrite just-created '{}' with '{}'",
                targetpath.display(),
                srcpath.display()
            );
            all_successful = false;
            continue;
        }

        // 创建链接
        if let Err(e) = ln_link(srcpath, &targetpath, settings) {
            ct_show_error!("{}", e);
            all_successful = false;
        }

        linked_destinations.insert(targetpath);
    }

    if all_successful {
        Ok(())
    } else {
        Err(LnError::SomeLinksFailed.into())
    }
}

fn ln_target_is_dir(path: &Path, settings: &LnSettings) -> bool {
    if settings.is_no_dereference {
        fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
    } else {
        path.is_dir()
    }
}

/// 构建目标文件的路径。
fn build_target_path(src: &Path, target_dir: &Path, _settings: &LnSettings) -> CTResult<PathBuf> {
    build_normal_target_path(src, target_dir)
}

/// 构建普通目标路径。
fn build_normal_target_path(src: &Path, target_dir: &Path) -> CTResult<PathBuf> {
    let basename = src.file_name().unwrap_or_else(|| src.as_os_str());
    Ok(target_dir.join(basename))
}

fn relative_path<'a>(src: &'a Path, dst: &Path) -> Cow<'a, Path> {
    if let Ok(src_abs) = canonicalize(src, MissingHandling::Missing, ResolveMode::Physical) {
        if let Ok(dst_abs) = canonicalize(
            dst.parent().unwrap(),
            MissingHandling::Missing,
            ResolveMode::Physical,
        ) {
            return make_path_relative_to(src_abs, dst_abs).into();
        }
    }
    src.into()
}

/// 检查两个路径是否指向同一个物理文件
fn is_same_file(path1: &Path, path2: &Path, settings: &LnSettings) -> bool {
    // 1. 获取目标(dst)的 metadata：永远使用 lstat，因为我们要覆盖的是目标自身，而不是它指向的东西
    let meta2 = match fs::symlink_metadata(path2) {
        Ok(m) => m,
        Err(_) => return false, // 目标不存在，不可能冲突
    };

    // 2. 获取源(src)的 metadata：根据模式决定是否穿透符号链接
    let meta1 = if settings.is_symbolic || settings.is_logical {
        // -s 符号链接 或 -L 逻辑硬链接：尝试穿透。
        // 如果源是一个悬空的坏链接，这里会 err，直接判定为无冲突，放行覆盖。
        match fs::metadata(path1) {
            Ok(m) => m,
            Err(_) => return false,
        }
    } else {
        // 默认硬链接 (-P 物理模式)：不穿透
        match fs::symlink_metadata(path1) {
            Ok(m) => m,
            Err(_) => return false,
        }
    };

    let is_same_inode = {
        #[cfg(unix)]
        {
            meta1.dev() == meta2.dev() && meta1.ino() == meta2.ino()
        }
        #[cfg(not(unix))]
        {
            if let (Ok(c1), Ok(c2)) = (fs::canonicalize(path1), fs::canonicalize(path2)) {
                c1 == c2
            } else {
                false
            }
        }
    };

    if is_same_inode {
        // 如果将要覆盖或备份目标文件
        let will_replace = settings.overwrite == OverwriteMode::Force
            || settings.overwrite == OverwriteMode::Interactive
            || settings.backup != CtBackupMode::NoBackup;

        // 如果没有开启替换，同数据块直接视为同文件，拒绝操作
        if !will_replace {
            return true;
        }

        // 如果开启了替换（如 -f），则检查它们的绝对规范路径。
        // 路径不同（如 f 和 d/f，属于不同硬链接），放行覆盖。
        // 路径相同（如 a 和 ./a），阻断自杀式覆盖！
        if let (Ok(c1), Ok(c2)) = (fs::canonicalize(path1), fs::canonicalize(path2)) {
            return c1 == c2;
        }

        return true; // 获取路径失败时，保守拒绝操作以防数据丢失
    }

    false
}

/// 创建链接（符号链接或硬链接）。
///
/// # 参数
/// * `src` - 源文件路径
/// * `dst` - 目标文件路径
/// * `settings` - 链接设置
fn ln_link(src: &Path, dst: &Path, settings: &LnSettings) -> CTResult<()> {
    // 1. 解析源路径
    let source = resolve_source_path(src, dst, settings)?;

    if is_same_file(&source, dst, settings) {
        return Err(LnError::SameFile(source.into_owned(), dst.to_path_buf()).into());
    }

    // GNU ln validates hard-link sources before any destructive destination handling.
    if !settings.is_symbolic {
        validate_hard_link_source(&source, settings)?;
    }

    // 2. 处理备份和覆盖，并检查目标文件的备份是否存在，如果存在则删除备份文件
    let backup_path = handle_backup_and_overwrite(src, dst, settings)?;

    // 3. 创建链接
    create_link(&source, dst, settings)?;

    // 4. 打印详细信息
    print_link_info(dst, &source, backup_path, settings);

    Ok(())
}

/// 解析源路径，处理相对路径。
fn resolve_source_path<'a>(
    src: &'a Path,
    dst: &Path,
    settings: &LnSettings,
) -> CTResult<Cow<'a, Path>> {
    if settings.is_relative {
        Ok(relative_path(src, dst))
    } else {
        Ok(src.into())
    }
}

/// 处理备份和覆盖模式。
fn handle_backup_and_overwrite(
    src: &Path,
    dst: &Path,
    settings: &LnSettings,
) -> CTResult<Option<PathBuf>> {
    // 使用 symlink_metadata 准确判断文件或悬空链接是否存在
    let Ok(dst_metadata) = fs::symlink_metadata(dst) else {
        return Ok(None);
    };

    let will_replace =
        settings.overwrite != OverwriteMode::NoClobber || settings.backup != CtBackupMode::NoBackup;

    // GNU ln rejects directories before prompting, backing up, or unlinking.
    if will_replace && dst_metadata.is_dir() {
        return Err(LnError::CannotOverwriteDirectory(dst.to_path_buf()).into());
    }

    if settings.overwrite == OverwriteMode::Interactive && !prompt_overwrite(dst) {
        return Err(LnError::SomeLinksFailed.into());
    }

    let backup_path = generate_backup_path(dst, settings)?;
    if let Some(ref backup) = backup_path {
        if fs::symlink_metadata(backup).is_ok() {
            let _ = fs::remove_file(backup); // 忽略错误，交由系统处理
        }
        rename_backup(dst, backup)?;
    }

    handle_overwrite_mode(dst, src, settings)?;
    Ok(backup_path)
}

fn prompt_overwrite(dst: &Path) -> bool {
    eprint!("{}: replace {}? ", ctcore::ct_util_name(), dst.quote());
    ctcore::ct_crash_if_err!(1, std::io::stderr().flush());
    read_yes_response()
}

fn read_yes_response() -> bool {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut response = Vec::new();

    match stdin.read_until(b'\n', &mut response) {
        Ok(0) | Err(_) => false,
        Ok(_) => {
            if response.last() == Some(&b'\n') {
                response.pop();
            }
            response_is_yes(&response)
        }
    }
}

#[cfg(unix)]
fn response_is_yes(response: &[u8]) -> bool {
    let mut nul_terminated = Vec::with_capacity(response.len() + 1);
    nul_terminated.extend_from_slice(response);
    nul_terminated.push(0);
    unsafe { rpmatch(nul_terminated.as_ptr().cast()) > 0 }
}

#[cfg(not(unix))]
fn response_is_yes(response: &[u8]) -> bool {
    matches!(response.first(), Some(b'y' | b'Y'))
}

/// 生成备份路径。
fn generate_backup_path(dst: &Path, settings: &LnSettings) -> CTResult<Option<PathBuf>> {
    Ok(match settings.backup {
        CtBackupMode::NoBackup => None,
        CtBackupMode::SimpleBackup => Some(ln_simple_backup_path(dst, &settings.suffix)),
        CtBackupMode::NumberedBackup => Some(ln_numbered_backup_path(dst)),
        CtBackupMode::ExistingBackup => Some(ln_existing_backup_path(dst, &settings.suffix)),
    })
}

/// 重命名为备份文件。
fn rename_backup(dst: &Path, backup: &Path) -> CTResult<()> {
    fs::rename(dst, backup).map_err_context(|| format!("cannot backup {}", dst.quote()))
}

/// 处理覆盖模式。
fn handle_overwrite_mode(dst: &Path, _src: &Path, settings: &LnSettings) -> CTResult<()> {
    match settings.overwrite {
        OverwriteMode::NoClobber => Ok(()),
        OverwriteMode::Interactive | OverwriteMode::Force => {
            // unlink(2), as used by GNU ln, cannot remove a directory even if
            // the destination changes after the metadata check above.
            if fs::symlink_metadata(dst).is_ok() {
                let _ = fs::remove_file(dst);
            }
            Ok(())
        }
    }
}

/// 创建链接（符号或硬链接）。
fn create_link(source: &Path, dst: &Path, settings: &LnSettings) -> CTResult<()> {
    if settings.is_symbolic {
        symlink(source, dst).map_err_context(|| {
            format!(
                "failed to create symbolic link {} => {}",
                source.quote(),
                dst.quote()
            )
        })
    } else {
        create_hard_link(source, dst, settings)
    }
}

/// 创建硬链接，处理符号链接解析。
fn create_hard_link(source: &Path, dst: &Path, settings: &LnSettings) -> CTResult<()> {
    let resolved_source = validate_hard_link_source(source, settings)?;

    fs::hard_link(&resolved_source, dst).map_err(|err| {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            LnError::HardLinkAlreadyExists(dst.to_path_buf()).into()
        } else {
            err.map_err_context(|| {
                format!(
                    "failed to create hard link {} => {}",
                    source.quote(),
                    dst.quote()
                )
            }) as Box<dyn CTError>
        }
    })
}

fn validate_hard_link_source(source: &Path, settings: &LnSettings) -> CTResult<PathBuf> {
    let resolved_source = if settings.is_logical && source.is_symlink() {
        fs::canonicalize(source)
            .map_err_context(|| format!("failed to access {}", source.quote()))?
    } else {
        source.to_path_buf()
    };

    // 使用 symlink_metadata 来判断是否为目录。
    // 这保证了在使用 -P (物理模式) 时，指向目录的符号链接不会被误判为目录，
    // 但如果路径带有尾部斜杠（如 link-to-dir/），系统底层会自动解析它，从而正确拦截。
    let is_dir = fs::symlink_metadata(&resolved_source)
        .map_err_context(|| format!("failed to access {}", source.quote()))?
        .is_dir();

    // 检查是否是目录硬链接请求
    if is_dir {
        if !settings.is_directory {
            // 抛出正确的、专属的目录硬链接错误
            return Err(LnError::HardLinkToDirectory(source.to_path_buf()).into());
        }

        // 检查是否具有root权限
        if !is_root() {
            return Err(LnError::NonRootPermission.into());
        }
    }

    Ok(resolved_source)
}

/// 检查当前用户是否为root
fn is_root() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// 打印链接详细信息。
fn print_link_info(dst: &Path, source: &Path, backup_path: Option<PathBuf>, settings: &LnSettings) {
    if settings.is_verbose {
        print!("{} => {}", dst.quote(), source.quote());
        if let Some(path) = backup_path {
            println!(" (backup: {})", path.quote());
        } else {
            println!();
        }
    }
}

fn ln_simple_backup_path(path: &Path, suffix: &OsStr) -> PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(suffix);
    PathBuf::from(p)
}

fn ln_numbered_backup_path(path: &Path) -> PathBuf {
    let mut i: u64 = 1;
    loop {
        let suffix = OsString::from(format!(".~{i}~"));
        let new_path = ln_simple_backup_path(path, &suffix);
        if !new_path.exists() {
            return new_path;
        }
        i += 1;
    }
}

fn ln_existing_backup_path(path: &Path, suffix: &OsStr) -> PathBuf {
    let test_path = ln_simple_backup_path(path, OsStr::new(".~1~"));
    if test_path.exists() {
        return ln_numbered_backup_path(path);
    }
    ln_simple_backup_path(path, suffix)
}

#[cfg(windows)]
pub fn symlink<P1: AsRef<Path>, P2: AsRef<Path>>(src: P1, dst: P2) -> std::io::Result<()> {
    if src.as_ref().is_dir() {
        symlink_dir(src, dst)
    } else {
        symlink_file(src, dst)
    }
}

fn ln_determine_backup_mode(matches: &ArgMatches) -> CTResult<CtBackupMode> {
    let suffix_enables_backup = matches
        .get_one::<OsString>(ct_backup_control::arguments::OPT_SUFFIX)
        .is_some();
    let has_backup = matches.contains_id(ct_backup_control::arguments::OPT_BACKUP)
        || matches.get_flag(ct_backup_control::arguments::OPT_BACKUP_NO_ARG)
        || suffix_enables_backup;
    let has_explicit_control = matches
        .get_one::<String>(ct_backup_control::arguments::OPT_BACKUP)
        .is_some();

    if has_backup
        && !has_explicit_control
        && let Some(control) = std::env::var_os("VERSION_CONTROL")
        && control.to_str().is_none()
    {
        return Err(ct_backup_control::CtBackupError::InvalidArgument(
            control.to_string_lossy().into_owned(),
            "$VERSION_CONTROL".to_owned(),
        )
        .into());
    }

    ct_backup_control::determine_backup_mode_with_activation(matches, suffix_enables_backup)
}

fn ln_determine_backup_suffix(matches: &ArgMatches) -> OsString {
    matches
        .get_one::<OsString>(ct_backup_control::arguments::OPT_SUFFIX)
        .cloned()
        .or_else(|| std::env::var_os("SIMPLE_BACKUP_SUFFIX"))
        .unwrap_or_else(|| OsString::from("~"))
}

impl LnSettings {
    /// 从命令行参数创建新的 LnSettings 实例。
    ///
    /// # 参数
    /// * `matches` - 命令行参数匹配结果
    ///
    /// # 返回值
    /// 返回 `CTResult<Self>`，包含配置实例或错误
    pub fn new(matches: &ArgMatches) -> CTResult<Self> {
        /* the list of files */

        let symbolic = matches.get_flag(lnoptions::LN_SYMBOLIC);

        let overwrite_mode = if matches.get_flag(lnoptions::LN_FORCE) {
            OverwriteMode::Force
        } else if matches.get_flag(lnoptions::LN_INTERACTIVE) {
            OverwriteMode::Interactive
        } else {
            OverwriteMode::NoClobber
        };

        let backup_mode = ln_determine_backup_mode(matches)?;
        let backup_suffix = ln_determine_backup_suffix(matches);

        // When we have "-L" or "-L -P", false otherwise
        let logical = matches.get_flag(lnoptions::LN_LOGICAL);

        Ok(Self {
            overwrite: overwrite_mode,
            backup: backup_mode,
            suffix: backup_suffix,
            is_symbolic: symbolic,
            is_logical: logical,
            is_relative: matches.get_flag(lnoptions::LN_RELATIVE),
            target_dir: matches
                .get_one::<PathBuf>(lnoptions::LN_TARGET_DIRECTORY)
                .cloned(),
            is_no_target_dir: matches.get_flag(lnoptions::LN_NO_TARGET_DIRECTORY),
            is_no_dereference: matches.get_flag(lnoptions::LN_NO_DEREFERENCE),
            is_verbose: matches.get_flag(lnoptions::LN_VERBOSE),
            is_directory: matches.get_flag(lnoptions::LN_DIRECTORY),
        })
    }
}

impl Default for LnSettings {
    /// 创建默认的 LnSettings 实例。
    ///
    /// # 默认值
    /// - overwrite: NoClobber - 不覆盖现有文件
    /// - backup: NoBackup - 不创建备份
    /// - suffix: "" - 空备份后缀
    /// - is_symbolic: false - 创建硬链接
    /// - is_relative: false - 使用绝对路径
    /// - is_logical: false - 不跟随符号链接
    /// - target_dir: None - 无指定目标目录
    /// - is_no_target_dir: false - 正常处理目标目录
    /// - is_no_dereference: false - 正常解引用符号链接
    /// - is_verbose: false - 不显示详细信息
    /// - is_directory: false - 不允许创建目录硬链接
    fn default() -> Self {
        Self {
            overwrite: OverwriteMode::NoClobber,
            backup: CtBackupMode::NoBackup,
            suffix: OsString::new(),
            is_symbolic: false,
            is_relative: false,
            is_logical: false,
            target_dir: None,
            is_no_target_dir: false,
            is_no_dereference: false,
            is_verbose: false,
            is_directory: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs;
    use tempfile::tempdir;

    static BACKUP_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_ct_app_accepts_repeated_options() {
        for option in [
            "-ff", "-ii", "-vv", "-nn", "-TT", "-LL", "-PP", "-dd", "-bb",
        ] {
            ct_app()
                .try_get_matches_from(["ln", option, "source", "dest"])
                .unwrap_or_else(|error| panic!("repeated option {option} was rejected: {error}"));
        }

        ct_app()
            .try_get_matches_from(["ln", "-srr", "source", "dest"])
            .unwrap_or_else(|error| panic!("repeated option -r was rejected: {error}"));
    }

    #[test]
    fn test_force_and_interactive_last_option_wins() {
        let settings_for = |option| {
            let matches = ct_app()
                .try_get_matches_from(["ln", option, "source", "dest"])
                .unwrap();
            LnSettings::new(&matches).unwrap()
        };

        assert_eq!(settings_for("-fi").overwrite, OverwriteMode::Interactive);
        assert_eq!(settings_for("-if").overwrite, OverwriteMode::Force);
        assert_eq!(settings_for("-fif").overwrite, OverwriteMode::Force);
        assert_eq!(settings_for("-ifi").overwrite, OverwriteMode::Interactive);
    }

    #[test]
    fn test_yes_response_keeps_leading_whitespace() {
        assert!(response_is_yes(b"y"));
        assert!(response_is_yes(b"Y"));
        assert!(!response_is_yes(b" y"));
        assert!(!response_is_yes(b"\ty"));
    }

    #[test]
    fn test_symbolic_ignores_directory_option() {
        for options in ["-sd", "-ds", "-sF", "-Fs"] {
            let matches = ct_app()
                .try_get_matches_from(["ln", options, "missing", "link"])
                .unwrap_or_else(|error| panic!("options {options} were rejected: {error}"));
            let settings = LnSettings::new(&matches).unwrap();

            assert!(settings.is_symbolic);
            assert!(settings.is_directory);
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_non_utf8_operands() {
        use std::os::unix::ffi::OsStringExt;
        use std::os::unix::fs::MetadataExt as _;

        let temp = tempdir().unwrap();
        let positional_dir = temp.path().join("positional");
        fs::create_dir(&positional_dir).unwrap();
        let source = positional_dir.join(OsString::from_vec(b"source-\xff".to_vec()));
        let destination = positional_dir.join(OsString::from_vec(b"destination-\xfe".to_vec()));
        fs::write(&source, "data").unwrap();

        ln_main(
            [
                OsString::from("ln"),
                source.clone().into_os_string(),
                destination.clone().into_os_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(
            fs::metadata(&source).unwrap().ino(),
            fs::metadata(&destination).unwrap().ino()
        );

        let target_root = temp.path().join("target");
        fs::create_dir(&target_root).unwrap();
        let target_dir = target_root.join(OsString::from_vec(b"directory-\xfd".to_vec()));
        fs::create_dir(&target_dir).unwrap();
        let target_source = target_root.join(OsString::from_vec(b"source-\xfc".to_vec()));
        fs::write(&target_source, "data").unwrap();

        ln_main(
            [
                OsString::from("ln"),
                OsString::from("-t"),
                target_dir.clone().into_os_string(),
                target_source.clone().into_os_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        let target_link = target_dir.join(target_source.file_name().unwrap());
        assert_eq!(
            fs::metadata(&target_source).unwrap().ino(),
            fs::metadata(target_link).unwrap().ino()
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_non_utf8_backup_environment() {
        use std::os::unix::ffi::OsStringExt;

        let _guard = BACKUP_ENV_MUTEX
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        fn restore_var(name: &str, value: Option<OsString>) {
            unsafe {
                if let Some(value) = value {
                    std::env::set_var(name, value);
                } else {
                    std::env::remove_var(name);
                }
            }
        }

        let previous_suffix = std::env::var_os("SIMPLE_BACKUP_SUFFIX");
        let previous_control = std::env::var_os("VERSION_CONTROL");
        let raw_suffix = OsString::from_vec(b"-\xff".to_vec());
        let matches = ct_app()
            .try_get_matches_from(["ln", "-b", "source", "dest"])
            .unwrap();

        unsafe {
            std::env::set_var("SIMPLE_BACKUP_SUFFIX", &raw_suffix);
            std::env::remove_var("VERSION_CONTROL");
        }
        let suffix_settings = LnSettings::new(&matches);

        unsafe {
            std::env::set_var("VERSION_CONTROL", OsString::from_vec(vec![0xff]));
        }
        let control_result = LnSettings::new(&matches);

        restore_var("SIMPLE_BACKUP_SUFFIX", previous_suffix);
        restore_var("VERSION_CONTROL", previous_control);

        let suffix_settings = suffix_settings.unwrap();
        assert_eq!(suffix_settings.suffix, raw_suffix);
        assert_eq!(
            ln_simple_backup_path(Path::new("dest"), &suffix_settings.suffix)
                .into_os_string()
                .into_vec(),
            b"dest-\xff"
        );
        assert!(control_result.is_err());
    }

    #[test]
    fn test_repeated_backup_options_use_last_value() {
        let matches = ct_app()
            .try_get_matches_from([
                "ln",
                "-S",
                ".one",
                "-S",
                ".two",
                "--backup=simple",
                "--backup=numbered",
                "source",
                "dest",
            ])
            .unwrap();
        let settings = LnSettings::new(&matches).unwrap();

        assert_eq!(settings.suffix, OsString::from(".two"));
        assert_eq!(settings.backup, CtBackupMode::NumberedBackup);
    }

    #[test]
    fn test_suffix_option_enables_backup() {
        let _guard = BACKUP_ENV_MUTEX
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous_control = std::env::var_os("VERSION_CONTROL");
        unsafe { std::env::remove_var("VERSION_CONTROL") };

        let short_matches = ct_app()
            .try_get_matches_from(["ln", "-S", ".bak", "source", "dest"])
            .unwrap();
        let short_settings = LnSettings::new(&short_matches);
        let long_matches = ct_app()
            .try_get_matches_from(["ln", "--suffix=.bak", "source", "dest"])
            .unwrap();
        let long_settings = LnSettings::new(&long_matches);
        let disabled_matches = ct_app()
            .try_get_matches_from(["ln", "--backup=none", "-S", ".bak", "source", "dest"])
            .unwrap();
        let disabled_settings = LnSettings::new(&disabled_matches);

        unsafe {
            if let Some(control) = previous_control {
                std::env::set_var("VERSION_CONTROL", control);
            } else {
                std::env::remove_var("VERSION_CONTROL");
            }
        }

        for settings in [short_settings.unwrap(), long_settings.unwrap()] {
            assert_eq!(settings.backup, CtBackupMode::ExistingBackup);
            assert_eq!(settings.suffix, OsString::from(".bak"));
        }
        assert_eq!(disabled_settings.unwrap().backup, CtBackupMode::NoBackup);
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Ln;

        // 测试 name 方法
        assert_eq!(tool.name(), "ln");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("ln"));

        // 测试 execute 方法
        let args = vec![OsString::from("ln"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }
    #[test]
    fn test_link_files_in_dir() {
        let temp = tempdir().unwrap();

        // 创建源文件
        let source = temp.path().join("source.txt");
        fs::write(&source, "test content").unwrap();

        // 创建目标目录
        let target_dir = temp.path().join("target");
        fs::create_dir(&target_dir).unwrap();

        let settings = LnSettings {
            overwrite: OverwriteMode::Force,
            backup: CtBackupMode::NoBackup,
            suffix: OsString::new(),
            is_symbolic: true,
            is_relative: false,
            is_logical: false,
            target_dir: None,
            is_no_target_dir: false,
            is_no_dereference: false,
            is_verbose: true,
            is_directory: false,
        };

        // 测试基本链接创建
        let files = vec![source.clone()];
        assert!(link_files_in_dir(&files, &target_dir, &settings).is_ok());
        assert!(target_dir.join("source.txt").exists());

        // 测试多文件链接
        let source2 = temp.path().join("source2.txt");
        fs::write(&source2, "test content 2").unwrap();
        let files = vec![source.clone(), source2.clone()];
        assert!(link_files_in_dir(&files, &target_dir, &settings).is_ok());

        // 测试重复文件
        let files = vec![source.clone(), source.clone()];
        assert!(link_files_in_dir(&files, &target_dir, &settings).is_err());

        // 清理文件
        let _ = fs::remove_file(&source);
        let _ = fs::remove_file(&source2);
    }

    #[test]
    fn test_relative_path() {
        let temp = tempdir().unwrap();

        let src = temp.path().join("src/file.txt");
        let dst = temp.path().join("dst/link.txt");

        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::create_dir_all(dst.parent().unwrap()).unwrap();
        fs::write(&src, "test").unwrap();

        let rel_path = relative_path(&src, &dst);
        assert!(rel_path.to_str().unwrap().contains("../src/file.txt"));
    }

    #[test]
    fn test_link() {
        let temp = tempdir().unwrap();

        // 创建源文件
        let source = temp.path().join("source.txt");
        fs::write(&source, "test content").unwrap();
        let target = temp.path().join("target.txt");

        let settings = LnSettings {
            overwrite: OverwriteMode::Force,
            backup: CtBackupMode::SimpleBackup,
            suffix: OsString::from("~"),
            is_symbolic: true,
            is_relative: false,
            is_logical: false,
            target_dir: None,
            is_no_target_dir: false,
            is_no_dereference: false,
            is_verbose: true,
            is_directory: false,
        };

        // 测试基本链接
        assert!(ln_link(&source, &target, &settings).is_ok());
        assert!(target.exists());

        // 测试备份
        fs::write(&target, "old content").unwrap();
        assert!(ln_link(&source, &target, &settings).is_ok());
        assert!(target.exists());
        assert!(temp.path().join("target.txt~").exists());

        // 测试硬链接
        let settings = LnSettings {
            is_symbolic: false,
            ..settings
        };
        let hard_target = temp.path().join("hard_target.txt");
        assert!(ln_link(&source, &hard_target, &settings).is_ok());
        assert_eq!(fs::read(&source).unwrap(), fs::read(&hard_target).unwrap());

        // 清理文件
        let _ = fs::remove_file(&source);
        let _ = fs::remove_file(&target);
        let _ = fs::remove_file(&hard_target);
        let _ = fs::remove_file(temp.path().join("target.txt~"));
    }

    #[cfg(unix)]
    #[test]
    fn test_force_preserves_destination_for_invalid_hard_link_source() {
        let temp = tempdir().unwrap();
        let settings = LnSettings {
            overwrite: OverwriteMode::Force,
            ..LnSettings::default()
        };

        let missing_source = temp.path().join("missing");
        let missing_destination = temp.path().join("missing-destination");
        fs::write(&missing_destination, "old").unwrap();
        assert!(ln_link(&missing_source, &missing_destination, &settings).is_err());
        assert_eq!(fs::read_to_string(&missing_destination).unwrap(), "old");

        let directory_source = temp.path().join("directory-source");
        let directory_destination = temp.path().join("directory-destination");
        fs::create_dir(&directory_source).unwrap();
        fs::write(&directory_destination, "old").unwrap();
        assert!(ln_link(&directory_source, &directory_destination, &settings).is_err());
        assert_eq!(fs::read_to_string(&directory_destination).unwrap(), "old");

        let symbolic_destination = temp.path().join("symbolic-destination");
        fs::write(&symbolic_destination, "old").unwrap();
        let symbolic_settings = LnSettings {
            is_symbolic: true,
            ..settings
        };
        assert!(ln_link(&missing_source, &symbolic_destination, &symbolic_settings).is_ok());
        assert_eq!(fs::read_link(symbolic_destination).unwrap(), missing_source);
    }

    #[test]
    fn test_force_no_dereference_keeps_target_directory() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::write(&source, "data").unwrap();
        let settings = LnSettings {
            overwrite: OverwriteMode::Force,
            is_no_dereference: true,
            ..LnSettings::default()
        };

        let operand_dir = temp.path().join("operand-directory");
        fs::create_dir(&operand_dir).unwrap();
        assert!(ln_exec(&[source.clone(), operand_dir.clone()], &settings).is_ok());
        assert!(operand_dir.is_dir());
        assert_eq!(
            fs::read_to_string(operand_dir.join("source")).unwrap(),
            "data"
        );

        let target_source = temp.path().join("target-source");
        fs::write(&target_source, "target-data").unwrap();
        let target_dir = temp.path().join("target-directory");
        fs::create_dir(&target_dir).unwrap();
        let target_settings = LnSettings {
            target_dir: Some(target_dir.clone()),
            ..settings
        };
        assert!(ln_exec(&[target_source], &target_settings).is_ok());
        assert!(target_dir.is_dir());
        assert_eq!(
            fs::read_to_string(target_dir.join("target-source")).unwrap(),
            "target-data"
        );
    }

    #[test]
    fn test_no_overwrite_mode_replaces_a_directory() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::write(&source, "data").unwrap();

        for (name, overwrite, backup) in [
            ("force", OverwriteMode::Force, CtBackupMode::NoBackup),
            (
                "backup",
                OverwriteMode::NoClobber,
                CtBackupMode::SimpleBackup,
            ),
            (
                "interactive",
                OverwriteMode::Interactive,
                CtBackupMode::NoBackup,
            ),
        ] {
            let destination = temp.path().join(name);
            fs::create_dir(&destination).unwrap();
            let settings = LnSettings {
                overwrite,
                backup,
                suffix: OsString::from("~"),
                is_symbolic: true,
                is_no_target_dir: true,
                ..LnSettings::default()
            };

            let error = ln_exec(&[source.clone(), destination.clone()], &settings).unwrap_err();
            assert!(error.to_string().contains("cannot overwrite directory"));
            assert!(destination.is_dir());
            assert!(!ln_simple_backup_path(&destination, OsStr::new("~")).exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_ln_exec_dereferences_link_name_symlink_to_dir_by_default() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source.txt");
        fs::write(&source, "test content").unwrap();
        let target_dir = temp.path().join("targetdir");
        fs::create_dir(&target_dir).unwrap();
        let dirlink = temp.path().join("dirlink");
        std::os::unix::fs::symlink(&target_dir, &dirlink).unwrap();

        let settings = LnSettings::default();
        let files = vec![source.clone(), dirlink.clone()];

        assert!(ln_exec(&files, &settings).is_ok());
        assert!(target_dir.join("source.txt").exists());
        assert_eq!(fs::read_link(&dirlink).unwrap(), target_dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_ln_exec_no_dereference_treats_link_name_symlink_to_dir_as_file() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source.txt");
        fs::write(&source, "test content").unwrap();
        let target_dir = temp.path().join("targetdir");
        fs::create_dir(&target_dir).unwrap();
        let dirlink = temp.path().join("dirlink");
        std::os::unix::fs::symlink(&target_dir, &dirlink).unwrap();

        let settings = LnSettings {
            is_no_dereference: true,
            ..LnSettings::default()
        };
        let files = vec![source.clone(), dirlink.clone()];

        assert!(ln_exec(&files, &settings).is_err());
        assert!(!target_dir.join("source.txt").exists());
        assert_eq!(fs::read_link(&dirlink).unwrap(), target_dir);
    }

    #[test]
    fn test_backup_paths() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.txt");
        fs::write(&path, "test").unwrap();

        // 测试简单备份
        let backup = ln_simple_backup_path(&path, OsStr::new("~"));
        assert_eq!(backup.file_name().unwrap(), "test.txt~");

        // 测试编号备份
        let backup = ln_numbered_backup_path(&path);
        assert!(backup.to_str().unwrap().contains(".~1~"));

        // 测试已存在备份
        let backup = ln_existing_backup_path(&path, OsStr::new("~"));
        assert!(backup.to_str().unwrap().ends_with('~'));
    }

    #[test]
    fn test_overwrite_modes() {
        assert_ne!(OverwriteMode::Force, OverwriteMode::Interactive);
        assert_ne!(OverwriteMode::Interactive, OverwriteMode::NoClobber);
        assert_ne!(OverwriteMode::NoClobber, OverwriteMode::Force);
    }

    #[test]
    fn test_ln_exec() {
        let temp = tempdir().unwrap();

        // 创建源文件
        let source = temp.path().join("source.txt");
        fs::write(&source, "test content").unwrap();

        let target_dir = temp.path().join("target");
        fs::create_dir(&target_dir).unwrap();

        let settings = LnSettings {
            overwrite: OverwriteMode::Force,
            backup: CtBackupMode::NoBackup,
            suffix: OsString::new(),
            is_symbolic: true,
            is_relative: false,
            is_logical: false,
            target_dir: None,
            is_no_target_dir: false,
            is_no_dereference: false,
            is_verbose: false,
            is_directory: false,
        };

        // 测试第一种形式：直接链接到文件
        let files = vec![source.clone(), temp.path().join("link.txt")];
        assert!(ln_exec(&files, &settings).is_ok());
        assert!(temp.path().join("link.txt").exists());

        // 测试第二种形式：单文件链接到当前目录
        let files = vec![source.clone()];
        let settings = LnSettings {
            is_no_target_dir: false,
            ..settings
        };
        assert!(ln_exec(&files, &settings).is_ok());
        assert!(Path::new("source.txt").exists());

        // 测试第三种形式：多文件链接到目录
        let source2 = temp.path().join("source2.txt");
        fs::write(&source2, "test content 2").unwrap();
        let files = vec![source.clone(), source2.clone(), target_dir.clone()];
        assert!(ln_exec(&files, &settings).is_ok());
        assert!(target_dir.join("source.txt").exists());
        assert!(target_dir.join("source2.txt").exists());

        // 测试第四种形式：使用 -t 选项
        let new_target = temp.path().join("new_target");
        fs::create_dir(&new_target).unwrap();
        let settings = LnSettings {
            target_dir: Some(new_target.clone()),
            ..settings
        };
        let files = vec![source.clone(), source2.clone()];
        assert!(ln_exec(&files, &settings).is_ok());
        assert!(new_target.join("source.txt").exists());
        assert!(new_target.join("source2.txt").exists());

        // 测试错误情况

        // 测试缺少目标文件
        let files = vec![source.clone()];
        let settings = LnSettings {
            target_dir: None,
            is_no_target_dir: true,
            ..settings
        };
        assert!(ln_exec(&files, &settings).is_err());

        // 测试多余的操作数
        let files = vec![source.clone(), source2.clone(), source.clone()];
        assert!(ln_exec(&files, &settings).is_err());

        // 测试目标不是目录
        let not_dir = temp.path().join("not_dir.txt");
        fs::write(&not_dir, "not a directory").unwrap();
        let files = vec![source.clone(), source2.clone(), not_dir];
        let settings = LnSettings {
            is_no_target_dir: false,
            ..settings
        };
        assert!(ln_exec(&files, &settings).is_err());

        // 清理文件
        let _ = fs::remove_file(&source);
        let _ = fs::remove_file(&source2);
        let _ = fs::remove_file(Path::new("source.txt"));
        let _ = fs::remove_file(temp.path().join("link.txt"));
    }
}
