/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

// spell-checker:ignore (ToDO) srcpath targetpath EEXIST

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, FromIo};
use ctcore::ct_fs::{make_path_relative_to, paths_refer_to_same_file};
use ctcore::{
    ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_prompt_yes, ct_show_error,
};

use std::borrow::Cow;
use std::collections::HashSet;
use std::error::Error;
use std::ffi::OsString;
use std::fmt::Display;
use std::fs;

use ctcore::ct_backup_control::{self, CtBackupMode};
use ctcore::ct_fs::{MissingHandling, ResolveMode, canonicalize};
#[cfg(any(unix, target_os = "redox"))]
use std::os::unix::fs::symlink;
#[cfg(windows)]
use std::os::windows::fs::{symlink_dir, symlink_file};
use std::path::{Path, PathBuf};

pub struct LnSettings {
    overwrite: OverwriteMode,
    backup: CtBackupMode,
    suffix: String,
    is_symbolic: bool,
    is_relative: bool,
    is_logical: bool,
    target_dir: Option<String>,
    is_no_target_dir: bool,
    is_no_dereference: bool,
    is_verbose: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverwriteMode {
    NoClobber,
    Interactive,
    Force,
}

#[derive(Debug)]
enum LnError {
    TargetIsDirectory(PathBuf),
    SomeLinksFailed,
    SameFile(PathBuf, PathBuf),
    MissingDestination(PathBuf),
    ExtraOperand(OsString),
}

impl Display for LnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TargetIsDirectory(s) => write!(f, "target {} is not a directory", s.quote()),
            Self::SameFile(s, d) => {
                write!(f, "{} and {} are the same file", s.quote(), d.quote())
            }
            Self::SomeLinksFailed => Ok(()),
            Self::MissingDestination(s) => {
                write!(f, "missing destination file operand after {}", s.quote())
            }
            Self::ExtraOperand(s) => write!(
                f,
                "extra operand {}\nTry '{} --help' for more information.",
                s.quote(),
                ctcore::ct_execute_phrase()
            ),
        }
    }
}

impl Error for LnError {}

impl CTError for LnError {
    fn code(&self) -> i32 {
        1
    }
}

const LN_ABOUT: &str = ct_help_about!("ln.md");
const LN_USAGE: &str = ct_help_usage!("ln.md");
const LN_AFTER_HELP: &str = ct_help_section!("after help", "ln.md");

mod lnoptions {
    pub const LN_FORCE: &str = "force";
    //pub const DIRECTORY: &str = "directory";
    pub const LN_INTERACTIVE: &str = "interactive";
    pub const LN_NO_DEREFERENCE: &str = "no-dereference";
    pub const LN_SYMBOLIC: &str = "symbolic";
    pub const LN_LOGICAL: &str = "logical";
    pub const LN_PHYSICAL: &str = "physical";
    pub const LN_TARGET_DIRECTORY: &str = "target-directory";
    pub const LN_NO_TARGET_DIRECTORY: &str = "no-target-directory";
    pub const LN_RELATIVE: &str = "relative";
    pub const LN_VERBOSE: &str = "verbose";
}

static LN_ARG_FILES: &str = "files";

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    ln_main(args)
}
pub fn ln_main(args: impl ctcore::Args) -> CTResult<()> {
    let after_help = format!(
        "{}\n\n{}",
        LN_AFTER_HELP,
        ct_backup_control::CT_BACKUP_CONTROL_LONG_HELP
    );

    let matches = ct_app().after_help(after_help).try_get_matches_from(args)?;

    /* the list of files */

    let paths: Vec<PathBuf> = matches
        .get_many::<String>(LN_ARG_FILES)
        .unwrap()
        .map(PathBuf::from)
        .collect();

    let settings = LnSettings::new(&matches)?;

    ln_exec(&paths[..], &settings)
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(lnoptions::LN_FORCE)
            .short('f')
            .long(lnoptions::LN_FORCE)
            .help("remove existing destination files")
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_INTERACTIVE)
            .short('i')
            .long(lnoptions::LN_INTERACTIVE)
            .help("prompt whether to remove existing destination files")
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_NO_DEREFERENCE)
            .short('n')
            .long(lnoptions::LN_NO_DEREFERENCE)
            .help(
                "treat LINK_NAME as a normal file if it is a \
                    symbolic link to a directory",
            )
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_LOGICAL)
            .short('L')
            .long(lnoptions::LN_LOGICAL)
            .help("follow TARGETs that are symbolic links")
            .overrides_with(lnoptions::LN_PHYSICAL)
            .action(ArgAction::SetTrue),
        // Not implemented yet
        Arg::new(lnoptions::LN_PHYSICAL)
            .short('P')
            .long(lnoptions::LN_PHYSICAL)
            .help("make hard links directly to symbolic links")
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_SYMBOLIC)
            .short('s')
            .long(lnoptions::LN_SYMBOLIC)
            .help("make symbolic links instead of hard links")
            // override added for https://github.com/ctutils/coreutils/issues/2359
            .overrides_with(lnoptions::LN_SYMBOLIC)
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_TARGET_DIRECTORY)
            .short('t')
            .long(lnoptions::LN_TARGET_DIRECTORY)
            .help("specify the DIRECTORY in which to create the links")
            .value_name("DIRECTORY")
            .value_hint(clap::ValueHint::DirPath)
            .conflicts_with(lnoptions::LN_NO_TARGET_DIRECTORY),
        Arg::new(lnoptions::LN_NO_TARGET_DIRECTORY)
            .short('T')
            .long(lnoptions::LN_NO_TARGET_DIRECTORY)
            .help("treat LINK_NAME as a normal file always")
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_RELATIVE)
            .short('r')
            .long(lnoptions::LN_RELATIVE)
            .help("create symbolic links relative to link location")
            .requires(lnoptions::LN_SYMBOLIC)
            .action(ArgAction::SetTrue),
        Arg::new(lnoptions::LN_VERBOSE)
            .short('v')
            .long(lnoptions::LN_VERBOSE)
            .help("print name of each linked file")
            .action(ArgAction::SetTrue),
        Arg::new(LN_ARG_FILES)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath)
            .required(true)
            .num_args(1..),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(LN_ABOUT)
        .override_usage(ct_format_usage(LN_USAGE))
        .infer_long_args(true)
        .arg(ct_backup_control::arguments::backup())
        .arg(ct_backup_control::arguments::backup_no_args())
        .arg(ct_backup_control::arguments::suffix())
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
        return link_files_in_dir(files, &PathBuf::from(target_dir), settings);
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
        if files.len() > 2 || last_file.is_dir() {
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
    if !target_dir.is_dir() {
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

/// 构建目标文件的路径。
fn build_target_path(src: &Path, target_dir: &Path, settings: &LnSettings) -> CTResult<PathBuf> {
    if settings.is_no_dereference && matches!(settings.overwrite, OverwriteMode::Force) {
        handle_no_dereference_path(target_dir)
    } else {
        build_normal_target_path(src, target_dir)
    }
}

/// 处理 no_dereference 选项的目标路径。
fn handle_no_dereference_path(target_dir: &Path) -> CTResult<PathBuf> {
    if target_dir.is_symlink() {
        if target_dir.is_file() {
            let _ = fs::remove_file(target_dir);
        }
        if target_dir.is_dir() {
            let _ = fs::remove_dir(target_dir);
        }
    }
    Ok(target_dir.to_path_buf())
}

/// 构建普通目标路径。
fn build_normal_target_path(src: &Path, target_dir: &Path) -> CTResult<PathBuf> {
    match src.as_os_str().to_str() {
        Some(name) => {
            let basename = Path::new(name)
                .file_name()
                .map_or_else(|| name, |b| b.to_str().unwrap());
            Ok(target_dir.join(basename))
        }
        None => {
            ct_show_error!("cannot stat {}: No such file or directory", src.quote());
            Err(LnError::SomeLinksFailed.into())
        }
    }
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

/// 创建链接（符号链接或硬链接）。
///
/// # 参数
/// * `src` - 源文件路径
/// * `dst` - 目标文件路径
/// * `settings` - 链接设置
fn ln_link(src: &Path, dst: &Path, settings: &LnSettings) -> CTResult<()> {
    // 1. 解析源路径
    let source = resolve_source_path(src, dst, settings)?;

    // 2. 处理备份和覆盖
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
    if !dst.is_symlink() && !dst.exists() {
        return Ok(None);
    }

    let backup_path = generate_backup_path(dst, settings)?;
    if let Some(ref backup) = backup_path {
        rename_backup(dst, backup)?;
    }

    handle_overwrite_mode(dst, src, settings)?;
    Ok(backup_path)
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
fn handle_overwrite_mode(dst: &Path, src: &Path, settings: &LnSettings) -> CTResult<()> {
    match settings.overwrite {
        OverwriteMode::NoClobber => Ok(()),
        OverwriteMode::Interactive => {
            if !ct_prompt_yes!("replace {}?", dst.quote()) {
                return Err(LnError::SomeLinksFailed.into());
            }
            let _ = fs::remove_file(dst);
            Ok(())
        }
        OverwriteMode::Force => {
            if !dst.is_symlink() && paths_refer_to_same_file(src, dst, true) {
                return Err(LnError::SameFile(src.to_owned(), dst.to_owned()).into());
            }
            let _ = fs::remove_file(dst);
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
    let resolved_source = if settings.is_logical && source.is_symlink() {
        fs::canonicalize(source)
            .map_err_context(|| format!("failed to access {}", source.quote()))?
    } else {
        source.to_path_buf()
    };

    fs::hard_link(&resolved_source, dst).map_err_context(|| {
        format!(
            "failed to create hard link {} => {}",
            source.quote(),
            dst.quote()
        )
    })
}

/// 打印链接详细信息。
fn print_link_info(dst: &Path, source: &Path, backup_path: Option<PathBuf>, settings: &LnSettings) {
    if settings.is_verbose {
        print!("{} -> {}", dst.quote(), source.quote());
        if let Some(path) = backup_path {
            println!(" (backup: {})", path.quote());
        } else {
            println!();
        }
    }
}

fn ln_simple_backup_path(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.as_os_str().to_str().unwrap().to_owned();
    p.push_str(suffix);
    PathBuf::from(p)
}

fn ln_numbered_backup_path(path: &Path) -> PathBuf {
    let mut i: u64 = 1;
    loop {
        let new_path = ln_simple_backup_path(path, &format!(".~{i}~"));
        if !new_path.exists() {
            return new_path;
        }
        i += 1;
    }
}

fn ln_existing_backup_path(path: &Path, suffix: &str) -> PathBuf {
    let test_path = ln_simple_backup_path(path, ".~1~");
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

        let backup_mode = ct_backup_control::determine_backup_mode(matches)?;
        let backup_suffix = ct_backup_control::determine_backup_suffix(matches);

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
                .get_one::<String>(lnoptions::LN_TARGET_DIRECTORY)
                .map(String::from),
            is_no_target_dir: matches.get_flag(lnoptions::LN_NO_TARGET_DIRECTORY),
            is_no_dereference: matches.get_flag(lnoptions::LN_NO_DEREFERENCE),
            is_verbose: matches.get_flag(lnoptions::LN_VERBOSE),
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
    fn default() -> Self {
        Self {
            overwrite: OverwriteMode::NoClobber,
            backup: CtBackupMode::NoBackup,
            suffix: String::new(),
            is_symbolic: false,
            is_relative: false,
            is_logical: false,
            target_dir: None,
            is_no_target_dir: false,
            is_no_dereference: false,
            is_verbose: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

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
            suffix: String::new(),
            is_symbolic: true,
            is_relative: false,
            is_logical: false,
            target_dir: None,
            is_no_target_dir: false,
            is_no_dereference: false,
            is_verbose: true,
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
            suffix: String::from("~"),
            is_symbolic: true,
            is_relative: false,
            is_logical: false,
            target_dir: None,
            is_no_target_dir: false,
            is_no_dereference: false,
            is_verbose: true,
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

    #[test]
    fn test_backup_paths() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.txt");
        fs::write(&path, "test").unwrap();

        // 测试简单备份
        let backup = ln_simple_backup_path(&path, "~");
        assert_eq!(backup.file_name().unwrap(), "test.txt~");

        // 测试编号备份
        let backup = ln_numbered_backup_path(&path);
        assert!(backup.to_str().unwrap().contains(".~1~"));

        // 测试已存在备份
        let backup = ln_existing_backup_path(&path, "~");
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
            suffix: String::new(),
            is_symbolic: true,
            is_relative: false,
            is_logical: false,
            target_dir: None,
            is_no_target_dir: false,
            is_no_dereference: false,
            is_verbose: false,
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
            target_dir: Some(new_target.to_string_lossy().into_owned()),
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
