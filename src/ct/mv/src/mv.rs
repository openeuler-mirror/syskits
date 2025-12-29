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

/// mv 是 GNU 工具集中的一个命令，用于在类 Unix 系统（如 Linux 和 macOS）中移动文件和目录，或者重命名它们。
mod error;

use crate::opt_flags::ARG_FILES;
use crate::opt_flags::OPT_FORCE;
use crate::opt_flags::OPT_INTERACTIVE;
use crate::opt_flags::OPT_NO_CLOBBER;
use crate::opt_flags::OPT_NO_TARGET_DIRECTORY;
use crate::opt_flags::OPT_PROGRESS;
use crate::opt_flags::OPT_STRIP_TRAILING_SLASHES;
use crate::opt_flags::OPT_TARGET_DIRECTORY;
use crate::opt_flags::OPT_VERBOSE;

use clap::builder::ValueParser;
use clap::{crate_version, error::ErrorKind, Arg, ArgAction, ArgMatches, Command};
use ctcore::ct_backup_control::{self, source_is_target_backup};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{set_ct_exit_code, CTError, CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_fs::{
    are_hardlinks_or_one_way_symlink_to_same_file, are_hardlinks_to_same_file,
    path_ends_with_terminator,
};
#[cfg(all(unix, not(target_os = "macos")))]
use ctcore::ct_fsxattr;
use ctcore::ct_update_control;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix;
#[cfg(windows)]
use std::os::windows;
use std::path::{Path, PathBuf};

// 这些枚举（enums）被暴露出来是为了让其他项目（例如 nushell）能够创建一个 Options 值，这需要这些枚举。
pub use ctcore::{ct_backup_control::CtBackupMode, ct_update_control::CtUpdateMode};
use ctcore::{
    ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_prompt_yes, ct_show,
};

use fs_extra::dir::{
    get_size as dir_get_size, move_dir, move_dir_with_progress, CopyOptions as DirCopyOptions,
    TransitProcess, TransitProcessResult,
};

use crate::error::MvError;

/// `Options` 结构体代表了`mv`命令可能的配置选项。
/// 这个全面的结构集中了所有基于标志的选项，用于控制移动文件或目录的行为。
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MvOpts {
    /// 决定在遇到已存在文件时的覆盖策略。
    /// 可以设置为避免覆盖、在覆盖前提示，或者强制覆盖而不提示。
    /// '-n' '--no-clobber'
    /// '-i' '--interactive'
    /// '-f' '--force'
    pub overwrite: MvOverwriteMode,

    /// 管理在覆盖文件时的备份策略。
    /// 可以根据特定规则创建备份，或者完全不创建备份。
    /// `--backup[=CONTROL]`, `-b`
    pub backup: CtBackupMode,

    /// 指定备份文件的后缀名。
    /// 只有在启用备份创建时，此选项才相关。
    /// '-S' --suffix' backup suffix
    pub suffix: String,

    /// 控制如何处理文件更新，允许根据文件年龄或其他标准进行选择性更新。
    pub update: CtUpdateMode,

    /// 可选地指定移动操作的目标目录。
    /// 如果提供，移动操作将视此目录为目的地根目录。
    /// '-t, --target-directory=DIRECTORY'
    pub target_dir: Option<OsString>,

    /// 反转目标目录的解释，将其视为普通文件而不是目录。
    /// '-T, --no-target-directory
    pub no_target_dir: bool,

    /// 启用详细模式，在移动操作期间提供更详细的输出。

    /// '-v, --verbose'
    pub verbose: bool,

    /// 移动文件过程中删除目录路径中的尾部斜杠。

    /// '--strip-trailing-slashes'
    pub strip_slashes: bool,

    /// 在移动操作期间显示进度条，适用于长时间运行的移动操作。
    /// '-g, --progress'
    pub progress_bar: bool,
}

/// 表示遇到目标位置已存在文件时的可能行为。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MvOverwriteMode {
    /// 不覆盖已存在的文件，保护其完整性。
    NoClobber,
    /// 在覆盖前向用户提示，允许手动干预。
    Interactive,
    /// 不提示地覆盖已存在文件，无条件进行。
    Force,
}

const MV_ABOUT: &str = ct_help_about!("mv.md");
const MV_USAGE: &str = ct_help_usage!("mv.md");
const MV_AFTER_HELP: &str = ct_help_section!("after help", "mv.md");

mod opt_flags {
    pub const OPT_FORCE: &str = "force";
    pub const OPT_INTERACTIVE: &str = "interactive";
    pub const OPT_NO_CLOBBER: &str = "no-clobber";
    pub const OPT_STRIP_TRAILING_SLASHES: &str = "strip-trailing-slashes";
    pub const OPT_TARGET_DIRECTORY: &str = "target-directory";
    pub const OPT_NO_TARGET_DIRECTORY: &str = "no-target-directory";
    pub const OPT_VERBOSE: &str = "verbose";
    pub const OPT_PROGRESS: &str = "progress";
    pub const ARG_FILES: &str = "files";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    mv_main(args).map(|_| ())
}

pub fn mv_main(args: impl ctcore::Args) -> CTResult<()> {
    let mut command = ct_app();
    let args_match = command.try_get_matches_from_mut(args)?;

    let arg_files: Vec<OsString> = args_match
        .get_many::<OsString>(ARG_FILES)
        .unwrap_or_default()
        .cloned()
        .collect();

    if arg_files.len() == 1 && !args_match.contains_id(OPT_TARGET_DIRECTORY) {
        command.error(
            ErrorKind::TooFewValues,
            format!(
                "The argument '<{ARG_FILES}>...' requires at least 2 values, but only 1 was provided"
            ),
        )
        .exit();
    }

    let (mv_overwrite_mode, ct_backup_mode, ct_update_mode) = mv_modes_process(&args_match)?;

    if mv_overwrite_mode == MvOverwriteMode::NoClobber && ct_backup_mode != CtBackupMode::NoBackup {
        return Err(CTsageError::new(
            1,
            "options --backup and --no-clobber are mutually exclusive",
        ));
    }

    let ct_backup_suffix = ct_backup_control::determine_backup_suffix(&args_match);

    let target_directory = args_match
        .get_one::<OsString>(OPT_TARGET_DIRECTORY)
        .map(OsString::from);

    if let Some(ref maybe_dir) = target_directory {
        if !Path::new(&maybe_dir).is_dir() {
            return Err(MvError::TargetNotADirectory(maybe_dir.quote().to_string()).into());
        }
    }

    let opts = MvOpts {
        overwrite: mv_overwrite_mode,
        backup: ct_backup_mode,
        suffix: ct_backup_suffix,
        update: ct_update_mode,
        target_dir: target_directory,
        no_target_dir: args_match.get_flag(OPT_NO_TARGET_DIRECTORY),
        verbose: args_match.get_flag(OPT_VERBOSE),
        strip_slashes: args_match.get_flag(OPT_STRIP_TRAILING_SLASHES),
        progress_bar: args_match.get_flag(OPT_PROGRESS),
    };

    mv(&arg_files[..], &opts)
}

fn mv_modes_process(
    args_match: &ArgMatches,
) -> Result<(MvOverwriteMode, CtBackupMode, CtUpdateMode), Box<dyn CTError>> {
    let mv_overwrite_mode = mv_determine_overwrite_mode(args_match);
    let ct_backup_mode = ct_backup_control::determine_backup_mode(args_match)?;
    let ct_update_mode = ct_update_control::ct_determine_update_mode(args_match);
    Ok((mv_overwrite_mode, ct_backup_mode, ct_update_mode))
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = MV_ABOUT;
    let usage_description = ct_format_usage(MV_USAGE);
    let after_help = format!(
        "{MV_AFTER_HELP}\n\n{}",
        ct_backup_control::CT_BACKUP_CONTROL_LONG_HELP
    );

    let args = mv_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(after_help)
        .infer_long_args(true)
        .args(&args)
}

fn mv_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(OPT_FORCE)
            .short('f')
            .long(OPT_FORCE)
            .help("do not prompt before overwriting")
            .overrides_with_all([OPT_INTERACTIVE, OPT_NO_CLOBBER])
            .action(ArgAction::SetTrue),
        Arg::new(OPT_INTERACTIVE)
            .short('i')
            .long(OPT_INTERACTIVE)
            .help("prompt before override")
            .overrides_with_all([OPT_FORCE, OPT_NO_CLOBBER])
            .action(ArgAction::SetTrue),
        Arg::new(OPT_NO_CLOBBER)
            .short('n')
            .long(OPT_NO_CLOBBER)
            .help("do not overwrite an existing file")
            .overrides_with_all([OPT_FORCE, OPT_INTERACTIVE])
            .action(ArgAction::SetTrue),
        Arg::new(OPT_STRIP_TRAILING_SLASHES)
            .long(OPT_STRIP_TRAILING_SLASHES)
            .help("remove any trailing slashes from each SOURCE argument")
            .action(ArgAction::SetTrue),
        ct_backup_control::arguments::backup(),
        ct_backup_control::arguments::backup_no_args(),
        ct_backup_control::arguments::suffix(),
        ct_update_control::arguments::update(),
        ct_update_control::arguments::update_no_args(),
        Arg::new(OPT_TARGET_DIRECTORY)
            .short('t')
            .long(OPT_TARGET_DIRECTORY)
            .help("move all SOURCE arguments into DIRECTORY")
            .value_name("DIRECTORY")
            .value_hint(clap::ValueHint::DirPath)
            .conflicts_with(OPT_NO_TARGET_DIRECTORY)
            .value_parser(ValueParser::os_string()),
        Arg::new(OPT_NO_TARGET_DIRECTORY)
            .short('T')
            .long(OPT_NO_TARGET_DIRECTORY)
            .help("treat DEST as a normal file")
            .action(ArgAction::SetTrue),
        Arg::new(OPT_VERBOSE)
            .short('v')
            .long(OPT_VERBOSE)
            .help("explain what is being done")
            .action(ArgAction::SetTrue),
        Arg::new(OPT_PROGRESS)
            .short('g')
            .long(OPT_PROGRESS)
            .help(
                "Display a progress bar. \n\
                Note: this feature is not supported by GNU coreutils.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(ARG_FILES)
            .action(ArgAction::Append)
            .num_args(1..)
            .required(true)
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::AnyPath),
    ];
    args
}

/**
 * 根据命令行匹配结果确定文件覆盖模式。
 *
 * 该函数依据用户通过命令行提供的选项来决定在移动或重命名文件时如何处理已存在的目标文件。
 * 具体行为可能与GNU的mv命令有所不同，特别是在多个覆盖选项被指定时，默认采取更安全的策略。
 *
 * @param matches 命令行参数匹配结果的引用，用于检查用户指定的选项。
 * @return 返回一个MvOverwriteMode枚举值，指示如何处理文件覆盖情况。
 */
fn mv_determine_overwrite_mode(matches: &ArgMatches) -> MvOverwriteMode {
    // 确定文件覆盖模式的逻辑：
    // 首先检查是否通过命令行指定了不覆盖已有文件的选项；
    // 如果没有指定，则检查是否指定了交互式覆盖模式；
    // 如果以上选项都没有指定，则默认采用强制覆盖模式。
    if matches.get_flag(OPT_NO_CLOBBER) {
        MvOverwriteMode::NoClobber // 不覆盖已有文件
    } else if matches.get_flag(OPT_INTERACTIVE) {
        MvOverwriteMode::Interactive // 交互式覆盖模式
    } else {
        MvOverwriteMode::Force // 强制覆盖模式
    }
}

/**
 * 解析给定文件路径并根据选项调整路径格式。
 *
 * @param files 包含待处理文件路径的切片，路径可能是OsString格式。
 * @param opts 包含各种移动操作选项的引用，例如是否剥离路径中的斜杠。
 * @return 返回一个PathBuf类型的向量，其中包含了根据opts选项调整后的路径。
 */
fn mv_parse_paths(mv_files: &[OsString], mv_options: &MvOpts) -> Vec<PathBuf> {
    // 将文件路径切片中的每个路径转换为Path类型。
    let file_paths = mv_files.iter().map(Path::new);

    // 根据opts中的strip_slashes选项来处理路径。
    if mv_options.strip_slashes {
        // 如果需要剥离斜杠，则将路径分解为组件，并仅保留最后一个组件。
        file_paths
            .map(|p| p.components().as_path().to_owned())
            .collect::<Vec<PathBuf>>()
    } else {
        // 如果不需要剥离斜杠，则直接复制路径到PathBuf中。
        file_paths.map(|p| p.to_owned()).collect::<Vec<PathBuf>>()
    }
}

/**
 * 处理两个路径的移动或重命名操作。
 *
 * @param source_path 源路径的引用。
 * @param target_path 目标路径的引用。
 * @param mv_options 移动操作的选项。
 * @return 返回一个结果，成功时为()`，失败时为`CTResult`里的错误类型。
 */
fn mv_handle_two_paths(
    source_path: &Path,
    target_path: &Path,
    mv_options: &MvOpts,
) -> CTResult<()> {
    // 检查是否使用简单备份模式，并且目标是源的备份。如果是，则返回错误。
    if mv_options.backup == CtBackupMode::SimpleBackup
        && source_is_target_backup(source_path, target_path, &mv_options.suffix)
    {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "backing up {} might destroy source;  {} not moved",
                target_path.quote(),
                source_path.quote()
            ),
        )
        .into());
    }

    // 检查源路径是否无法获取符号链接元数据，如果是，返回相应的错误。
    if source_path.symlink_metadata().is_err() {
        return Err(if path_ends_with_terminator(source_path) {
            MvError::CannotStatNotADirectory(source_path.quote().to_string()).into()
        } else {
            MvError::NoSuchFile(source_path.quote().to_string()).into()
        });
    }

    // 检查源和目标是否指向同一个文件，且未设置备份。如果是，则返回相应的错误。
    if (source_path.eq(target_path)
        || are_hardlinks_to_same_file(source_path, target_path)
        || are_hardlinks_or_one_way_symlink_to_same_file(source_path, target_path))
        && mv_options.backup == CtBackupMode::NoBackup
    {
        return if source_path.eq(Path::new("."))
            || source_path.ends_with("/.")
            || source_path.is_file()
        {
            Err(MvError::SameFile(
                source_path.quote().to_string(),
                target_path.quote().to_string(),
            )
            .into())
        } else {
            Err(MvError::SelfSubdirectory(source_path.display().to_string()).into())
        };
    }

    // 检查目标路径是否为目录，且源与目标不一致、不是硬链接指向同一文件、没有设置no_target_dir选项。
    let target_is_directory = target_path.is_dir();
    let source_is_directory = source_path.is_dir();

    if path_ends_with_terminator(target_path)
        && (!target_is_directory && !source_is_directory)
        && !mv_options.no_target_dir
        && mv_options.update != CtUpdateMode::ReplaceIfOlder
    {
        return Err(MvError::FailedToAccessNotADirectory(target_path.quote().to_string()).into());
    }

    // 如果目标是目录
    if target_is_directory {
        // 如果设置了no_target_dir且源是目录，则尝试重命名。
        if mv_options.no_target_dir {
            if source_path.is_dir() {
                mv_rename(source_path, target_path, mv_options, None).map_err_context(|| {
                    format!(
                        "cannot move {} to {}",
                        source_path.quote(),
                        target_path.quote()
                    )
                })
            } else {
                Err(MvError::DirectoryToNonDirectory(target_path.quote().to_string()).into())
            }
            // 检查源和目标是否包含相同的子目录/目录，以避免移动到自身的情况。
        } else if target_path.starts_with(source_path) {
            Err(MvError::SelfTargetSubdirectory(
                source_path.display().to_string(),
                target_path.display().to_string(),
            )
            .into())
        } else {
            // 将文件移动到目录中。
            move_files_into_dir(&[source_path.to_path_buf()], target_path, mv_options)
        }
        // 如果目标存在且源是目录
    } else if target_path.exists() && source_path.is_dir() {
        // 根据是否覆盖选项，处理交互式询问或直接返回错误。
        match mv_options.overwrite {
            MvOverwriteMode::NoClobber => return Ok(()),
            MvOverwriteMode::Interactive => {
                if !ct_prompt_yes!("overwrite {}? ", target_path.quote()) {
                    return Err(io::Error::new(io::ErrorKind::Other, "").into());
                }
            }
            MvOverwriteMode::Force => {}
        };
        Err(MvError::NonDirectoryToDirectory(
            source_path.quote().to_string(),
            target_path.quote().to_string(),
        )
        .into())
        // 默认情况：尝试重命名或移动文件。
    } else {
        mv_rename(source_path, target_path, mv_options, None)
            .map_err(|e| CtSimpleError::new(1, format!("{e}")))
    }
}

/**
 * 处理多个路径，将它们移动到一个目标目录中。
 *
 * @param paths 一个包含要移动的文件或目录路径的slice，最后一个路径被视为目标目录。
 * @param opts 移动操作的选项，例如是否禁止目标目录。
 * @return 返回一个结果，成功时为()`，失败时为`CTsageError`。
 */
fn mv_handle_multiple_paths(paths: &[PathBuf], opts: &MvOpts) -> CTResult<()> {
    // 当禁止目标目录选项启用时，如果有超过两个的路径参数，则报错。
    if opts.no_target_dir {
        return Err(CTsageError::new(
            1,
            format!("mv: extra operand {}", paths[2].quote()),
        ));
    }
    // 获取目标目录路径和源路径。
    let target_dir = paths.last().unwrap();
    let sources = &paths[..paths.len() - 1];

    // 将源文件或目录移动到目标目录。
    move_files_into_dir(sources, target_dir, opts)
}

/// 执行mv命令。此命令将'source'移动到'target'，其中'target'是一个目录。如果'target'不存在，
/// 并且'source'是一个单个文件或目录，则'source'将被重命名为'target'。
pub fn mv(files: &[OsString], mv_options: &MvOpts) -> CTResult<()> {
    // 解析源文件和目标路径
    let file_paths = mv_parse_paths(files, mv_options);

    // 如果指定了目标目录，则将文件移动到该目录下
    if let Some(ref name) = mv_options.target_dir {
        return move_files_into_dir(&file_paths, &PathBuf::from(name), mv_options);
    }

    // 根据路径数量，分别处理两个路径或多个路径的情况
    match file_paths.len() {
        2 => mv_handle_two_paths(&file_paths[0], &file_paths[1], mv_options),
        _ => mv_handle_multiple_paths(&file_paths, mv_options),
    }
}

#[allow(clippy::cognitive_complexity)]
/**
 * 将多个文件移动到指定的目标目录。
 *
 * @param files 要移动的文件路径集合。
 * @param target_dir 目标目录路径。
 * @param options 移动文件时的选项。
 * @return 返回一个结果，成功时为()`，失败时为`MvError`。
 */
fn move_files_into_dir(
    mv_files: &[PathBuf],
    target_directory: &Path,
    mv_opts: &MvOpts,
) -> CTResult<()> {
    // 用于存储已移动文件的目标路径，避免重复移动
    let mut moved_dests: HashSet<PathBuf> = HashSet::with_capacity(mv_files.len());

    // 检查目标路径是否为目录
    if !target_directory.is_dir() {
        return Err(MvError::NotADirectory(target_directory.quote().to_string()).into());
    }

    // 获取目标目录的规范路径
    let canonicalize_target_dir = target_directory
        .canonicalize()
        .unwrap_or_else(|_| target_directory.to_path_buf());

    // 根据选项决定是否创建进度条
    let multi_progress = mv_opts.progress_bar.then(MultiProgress::new);

    // 如果移动多个文件，创建进度条来跟踪进度
    let progress = if let Some(ref multi_progress) = multi_progress {
        if mv_files.len() > 1 {
            Some(multi_progress.add(
                ProgressBar::new(mv_files.len().try_into().unwrap()).with_style(
                    ProgressStyle::with_template("moving {msg} {wide_bar} {pos}/{len}").unwrap(),
                ),
            ))
        } else {
            None
        }
    } else {
        None
    };

    // 遍历所有要移动的文件
    for source_path in mv_files {
        // 如果设置了进度条，更新进度条信息
        if let Some(ref pb) = progress {
            pb.set_message(source_path.to_string_lossy().to_string());
        }

        // 确定目标路径
        let targetpath = match source_path.file_name() {
            Some(name) => target_directory.join(name),
            None => {
                ct_show!(MvError::NoSuchFile(source_path.quote().to_string()));
                continue;
            }
        };

        // 检查是否已存在相同目标路径的文件，并根据备份选项处理
        if moved_dests.contains(&targetpath) && mv_opts.backup != CtBackupMode::NumberedBackup {
            // 如果目标文件是此mv调用中已创建的，不覆盖
            ct_show!(CtSimpleError::new(
                1,
                format!(
                    "will not overwrite just-created '{}' with '{}'",
                    targetpath.display(),
                    source_path.display()
                ),
            ));
            continue;
        }

        // 检查是否尝试将目录移动到自身
        if let Ok(canonical_source) = source_path.canonicalize() {
            if canonical_source == canonicalize_target_dir {
                // 用户尝试将目录移动到其自身子目录，显示警告并继续移动文件
                ct_show!(CtSimpleError::new(
                    1,
                    format!(
                        "cannot move '{}' to a subdirectory of itself, '{}/{}'",
                        source_path.display(),
                        target_directory.display(),
                        canonicalize_target_dir.components().last().map_or_else(
                            || target_directory.display().to_string(),
                            |dir| { PathBuf::from(dir.as_os_str()).display().to_string() }
                        )
                    )
                ));
                continue;
            }
        }

        // 尝试重命名文件（即移动文件），根据结果进行相应处理
        match mv_rename(source_path, &targetpath, mv_opts, multi_progress.as_ref()) {
            Err(err) if err.to_string().is_empty() => set_ct_exit_code(1),
            Err(err) => {
                let err = err.map_err_context(|| {
                    format!(
                        "cannot move {} to {}",
                        source_path.quote(),
                        targetpath.quote()
                    )
                });
                match multi_progress {
                    Some(ref pb) => pb.suspend(|| ct_show!(err)),
                    None => ct_show!(err),
                };
            }
            Ok(()) => (),
        }
        // 更新进度条
        if let Some(ref pb) = progress {
            pb.inc(1);
        }
        // 将目标路径加入到已移动文件的集合中
        moved_dests.insert(targetpath.clone());
    }
    // 移动全部文件完成后，返回成功
    Ok(())
}

/**
 * 重命名文件或目录。
 *
 * @param from_path 原始路径。
 * @param to_path 目标路径。
 * @param options 移动选项，包含更新模式、覆盖模式等。
 * @param multi_progress 多重进度条，用于显示进度。
 * @return io::Result<()>，操作成功返回Ok(())，失败返回Err()。
 */
fn mv_rename(
    from_path: &Path,
    to_path: &Path,
    options: &MvOpts,
    multi_progress: Option<&MultiProgress>,
) -> io::Result<()> {
    let mut backup_path = None;

    // 如果目标路径已存在，根据更新和覆盖选项进行处理
    if to_path.exists() {
        // 根据更新模式判断是否应该跳过重命名
        if options.update == CtUpdateMode::ReplaceIfOlder
            && options.overwrite == MvOverwriteMode::Interactive
        {
            // 当目标文件存在且更新模式为ReplaceIfOlder和交互式覆盖时，不进行任何操作
            return Ok(());
        }

        if options.update == CtUpdateMode::ReplaceNone {
            // 如果设置为不替换，直接返回成功
            return Ok(());
        }

        // 检查文件是否更旧，如果是，则不进行操作
        if (options.update == CtUpdateMode::ReplaceIfOlder)
            && fs::metadata(from_path)?.modified()? <= fs::metadata(to_path)?.modified()?
        {
            return Ok(());
        }

        // 根据覆盖模式处理目标文件已存在的情况
        match options.overwrite {
            MvOverwriteMode::NoClobber => {
                // 如果设置为不覆盖，返回错误
                let err_msg = format!("not replacing {}", to_path.quote());
                return Err(io::Error::new(io::ErrorKind::Other, err_msg));
            }
            MvOverwriteMode::Interactive => {
                // 如果设置为交互式覆盖，询问用户是否覆盖
                if !ct_prompt_yes!("overwrite {}?", to_path.quote()) {
                    return Err(io::Error::new(io::ErrorKind::Other, ""));
                }
            }
            MvOverwriteMode::Force => {
                // 如果设置为强制覆盖，不进行额外操作
                {}
            }
        };

        // 获取备份路径
        backup_path = ct_backup_control::get_backup_path(options.backup, to_path, &options.suffix);
        if let Some(ref backup_path) = backup_path {
            // 如果存在备份路径，则将目标文件重命名为备份路径
            mv_rename_with_fallback(to_path, backup_path, multi_progress)?;
        }
    }

    // 处理目标路径是目录的情况
    if to_path.exists() && to_path.is_dir() {
        // 如果源路径也是目录，且目标目录非空，则返回错误
        if from_path.is_dir() {
            if is_empty_dir(to_path) {
                fs::remove_dir(to_path)?;
            } else {
                return Err(io::Error::new(io::ErrorKind::Other, "Directory not empty"));
            }
        }
    }

    // 执行重命名操作
    mv_rename_with_fallback(from_path, to_path, multi_progress)?;

    // 如果设置了详细模式，输出重命名信息
    if options.verbose {
        let message = match backup_path {
            Some(path) => format!(
                "renamed {} -> {} (backup: {})",
                from_path.quote(),
                to_path.quote(),
                path.quote()
            ),
            None => format!("renamed {} -> {}", from_path.quote(), to_path.quote()),
        };

        // 根据是否提供了多进度条实例，选择合适的输出方式
        match multi_progress {
            Some(pb) => pb.suspend(|| {
                println!("{message}");
            }),
            None => println!("{message}"),
        };
    }
    Ok(())
}
/// 尝试使用 `fs::rename` 更改文件或目录名称，如果失败，则尝试通过复制和删除来备份。
///
/// # 参数
/// - `from`: 指定原始路径。
/// - `to`: 指定目标路径。
/// - `multi_progress`: 可选，用于多进度条更新的 `MultiProgress` 实例，可用于显示复制进度。
///
/// # 返回值
/// 返回一个 `io::Result<()>`, 成功则为 `Ok(())`, 失败则为包含错误信息的 `Err`。
fn mv_rename_with_fallback(
    from: &Path,
    to: &Path,
    multi_progress: Option<&MultiProgress>,
) -> io::Result<()> {
    // 尝试直接重命名，如果失败则尝试备份方法。
    if fs::rename(from, to).is_err() {
        // 获取原始路径的元数据，不跟随符号链接。
        let symlink_metadata = from.symlink_metadata()?;
        let file_type = symlink_metadata.file_type();

        // 根据文件类型执行相应的备份策略。
        if file_type.is_symlink() {
            // 对符号链接执行特定的重命名策略。
            mv_rename_symlink_fallback(from, to)?;
        } else if file_type.is_dir() {
            // 如果目标路径存在，则删除该目录，以匹配 `fs::rename` 的行为。
            if to.exists() {
                fs::remove_dir_all(to)?;
            }
            // 配置目录复制选项。
            let dir_copy_opts = DirCopyOptions {
                copy_inside: true,
                ..DirCopyOptions::new()
            };

            // 尝试计算目录的总大小，用于进度条显示。
            let dir_total_size = dir_get_size(from).ok();

            // 根据是否提供了多进度条以及目录大小，创建或不创建进度条。
            let is_progress_bar = if let (Some(multi_progress), Some(total_size)) =
                (multi_progress, dir_total_size)
            {
                let bar = ProgressBar::new(total_size).with_style(
                    ProgressStyle::with_template(
                        "{msg}: [{elapsed_precise}] {wide_bar} {bytes:>7}/{total_bytes:7}",
                    )
                    .unwrap(),
                );

                Some(multi_progress.add(bar))
            } else {
                None
            };

            // 仅在非 macOS 的 Unix 系统上，收集源文件的扩展属性。
            #[cfg(all(unix, not(target_os = "macos")))]
            let fsxattrs = ct_fsxattr::ct_retrieve_xattrs(from)
                .unwrap_or_else(|_| std::collections::HashMap::new());

            // 使用进度条信息复制目录，如果未提供进度条，则无进度显示地复制。
            let result = if let Some(ref pb) = is_progress_bar {
                move_dir_with_progress(from, to, &dir_copy_opts, |process_info: TransitProcess| {
                    pb.set_position(process_info.copied_bytes);
                    pb.set_message(process_info.file_name);
                    TransitProcessResult::ContinueOrAbort
                })
            } else {
                move_dir(from, to, &dir_copy_opts)
            };

            // 在非 macOS 的 Unix 系统上，将收集到的扩展属性应用到目标文件。
            #[cfg(all(unix, not(target_os = "macos")))]
            ct_fsxattr::ct_apply_xattrs(to, fsxattrs).unwrap();

            // 处理复制过程中可能出现的错误。
            if let Err(err) = result {
                return match err.kind {
                    fs_extra::error::ErrorKind::PermissionDenied => Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Permission denied",
                    )),
                    _ => Err(io::Error::new(io::ErrorKind::Other, format!("{err:?}"))),
                };
            }
        } else {
            // 对于非目录类型的文件，在非 macOS Unix 系统上复制文件并保留扩展属性，其他情况下只复制文件。
            #[cfg(all(unix, not(target_os = "macos")))]
            fs::copy(from, to)
                .and_then(|_| ct_fsxattr::ct_copy_xattrs(&from, &to))
                .and_then(|_| fs::remove_file(from))?;
            #[cfg(any(target_os = "macos", not(unix)))]
            fs::copy(from, to).and_then(|_| fs::remove_file(from))?;
        }
    }
    // 如果重命名或备份成功，则返回成功结果。
    Ok(())
}

/// 将给定的符号链接移动到给定的目的地。在Windows上，悬挂的符号链接会返回错误。
#[inline]
fn mv_rename_symlink_fallback(from: &Path, to: &Path) -> io::Result<()> {
    // 读取符号链接指向的路径
    let symlink_points_to_path = fs::read_link(from)?;

    // 针对不同的操作系统，执行相应的重命名和删除操作
    #[cfg(unix)]
    {
        // 在Unix系统上创建一个新的符号链接并删除原始的符号链接
        unix::fs::symlink(symlink_points_to_path, to).and_then(|_| fs::remove_file(from))?;
    }
    #[cfg(windows)]
    {
        // 在Windows上，根据符号链接指向的路径是否存在以及是文件还是目录来创建相应的符号链接
        if symlink_points_to_path.exists() {
            if symlink_points_to_path.is_dir() {
                windows::fs::symlink_dir(&symlink_points_to_path, to)?;
            } else {
                windows::fs::symlink_file(&symlink_points_to_path, to)?;
            }
            // 删除原始的符号链接
            fs::remove_file(from)?;
        } else {
            // 如果符号链接指向的路径不存在，则返回一个自定义的错误
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "can't determine symlink type, since it is dangling",
            ));
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        // 如果不是Windows或Unix系统，则返回一个不支持符号链接的错误
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "your operating system does not support symlinks",
        ));
    }
    // 函数执行成功，返回Ok(())
    Ok(())
}

/**
 * 检查指定路径是否为空目录
 *
 * 该函数尝试读取指定路径下的内容，如果读取成功且目录为空，则返回`true`；如果读取失败或目录不为空，则返回`false`。
 *
 * @param path 指定的路径，类型为`&Path`，表示要检查的目录路径。
 * @return 返回一个`bool`值，如果目录为空则为`true`，否则为`false`。
 */
fn is_empty_dir(path: &Path) -> bool {
    // 尝试读取目录内容
    match fs::read_dir(path) {
        // 如果读取成功，检查内容是否为空
        Ok(contents) => contents.peekable().peek().is_none(),
        // 如果读取失败，认为目录不为空
        Err(_e) => false,
    }
}

#[cfg(unix)]
#[cfg(test)]
mod tests {

    #[cfg(test)]
    mod tests_mv_main {
        use crate::mv_main;

        use std::ffi::OsString;

        use std::fs;
        use std::fs::File;
        use std::io::Write;
        use tempfile::Builder;

        // 定义删除文件的函数
        fn delete_file(file_path: &str) -> Result<(), std::io::Error> {
            // 使用remove_file函数尝试删除文件
            fs::remove_file(file_path)?;
            Ok(())
        }

        #[test]
        fn test_mv_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_mv_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_mv_main_dir_to_dir() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
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

            let src_dir = sub_dir_path.to_str().unwrap();
            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_dir, dst_dir, "-f"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "--force"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_file() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                src_file,
                "test_mv_main_file_to_file",
                "--force",
            ];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));
            let _ = delete_file("test_mv_main_file_to_file");
            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_interactive() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "--interactive"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
        #[test]
        fn test_mv_main_file_to_dir_no_clobber() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "--no-clobber"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_strip_trailing_slashes() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                src_file,
                dst_dir,
                "--strip-trailing-slashes",
            ];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_backup() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "--backup=simple"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_u() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "-u"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_suffix() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "--suffix=.bak"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_update_none() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "--update=none"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_update_all() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "--update=all"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_update_older() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "--update=older"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_no_target_directory() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                src_file,
                dst_dir,
                "--no-target-directory",
            ];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_verbose() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "--verbose"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_file_to_dir_progress() {
            let temp_dir = Builder::new().prefix("test_mv_main_f").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            let dst_dir_path = temp_dir.path().join("dst_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let src_file = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let dst_dir = dst_dir_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), src_file, dst_dir, "--progress"];
            let result = mv_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }
    #[cfg(test)]
    mod tests_mv_app {
        use crate::ct_app;
        use std::ffi::OsString;

        use crate::opt_flags::{
            OPT_FORCE, OPT_INTERACTIVE, OPT_NO_CLOBBER, OPT_NO_TARGET_DIRECTORY, OPT_PROGRESS,
            OPT_STRIP_TRAILING_SLASHES, OPT_TARGET_DIRECTORY, OPT_VERBOSE,
        };
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_f() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-f"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_FORCE), Some(&true));
        }

        #[test]
        fn test_ct_app_force() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--force"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_FORCE), Some(&true));
        }

        #[test]
        fn test_ct_app_i() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-i"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<bool>(OPT_INTERACTIVE),
                Some(&true)
            );
        }

        #[test]
        fn test_ct_app_interactive() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--interactive"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<bool>(OPT_INTERACTIVE),
                Some(&true)
            );
        }

        #[test]
        fn test_ct_app_n() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-n"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_NO_CLOBBER), Some(&true));
        }

        #[test]
        fn test_ct_app_no_clobber() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--no-clobber"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_NO_CLOBBER), Some(&true));
        }

        #[test]
        fn test_ct_app_strip_trailing_slashes() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--strip-trailing-slashes"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<bool>(OPT_STRIP_TRAILING_SLASHES),
                Some(&true)
            );
        }

        #[test]
        fn test_ct_app_backup_simple() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--backup=simple"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_b_simple() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-b", "simple"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_s() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-S", ".bak"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_suffix() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--suffix=.bak"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_none() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--update=none"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_all() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--update=all"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_older() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--update=older"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_none() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-u", "none"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_all() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-u", "all"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_older() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-u", "older"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_t_directory() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-t", "target-directory"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<OsString>(OPT_TARGET_DIRECTORY)
                    .unwrap(),
                OPT_TARGET_DIRECTORY
            );
        }

        #[test]
        fn test_ct_app_target_directory() {
            let args = vec![
                ctcore::ct_util_name(),
                "a",
                "b",
                "--target-directory",
                "target-directory",
            ];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<OsString>(OPT_TARGET_DIRECTORY)
                    .unwrap(),
                OPT_TARGET_DIRECTORY
            );
        }

        #[test]
        fn test_ct_app_n_t_directory() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-T", "target-directory"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<bool>(OPT_NO_TARGET_DIRECTORY),
                Some(&true)
            );
        }

        #[test]
        fn test_ct_app_n_target_directory() {
            let args = vec![
                ctcore::ct_util_name(),
                "a",
                "b",
                "--no-target-directory",
                "target-directory",
            ];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result.unwrap().get_one::<bool>(OPT_NO_TARGET_DIRECTORY),
                Some(&true)
            );
        }

        #[test]
        fn test_ct_app_v() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-v"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_VERBOSE), Some(&true));
        }
        #[test]
        fn test_ct_app_verbose() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--verbose"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_VERBOSE), Some(&true));
        }

        #[test]
        fn test_ct_app_g() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "-g"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_PROGRESS), Some(&true));
        }
        #[test]
        fn test_ct_app_progress() {
            let args = vec![ctcore::ct_util_name(), "a", "b", "--progress"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_PROGRESS), Some(&true));
        }
    }
    #[cfg(test)]
    mod tests_mv_fun {
        use crate::{mv_parse_paths, MvOpts, MvOverwriteMode};
        use ctcore::ct_backup_control::CtBackupMode;
        use ctcore::ct_update_control::CtUpdateMode;

        use std::ffi::OsString;

        use std::path::PathBuf;

        fn create_test_opts(overwrite: MvOverwriteMode, strip_slashes: bool) -> MvOpts {
            MvOpts {
                overwrite,
                backup: CtBackupMode::NoBackup,
                suffix: "".to_string(),
                update: CtUpdateMode::ReplaceNone,
                target_dir: None,
                no_target_dir: false,
                verbose: false,
                strip_slashes,
                progress_bar: false,
            }
        }

        #[test]
        fn test_mv_parse_paths_with_strip_slashes() {
            let files = vec![
                OsString::from("/path/to/file1.txt"),
                OsString::from("/path/to/file2.txt"),
                OsString::from("/path/to/directory/"),
            ];
            let mv_options = create_test_opts(MvOverwriteMode::Interactive, true);

            let result = mv_parse_paths(&files, &mv_options);
            assert_eq!(
                result,
                vec![
                    PathBuf::from("/path/to/file1.txt"),
                    PathBuf::from("/path/to/file2.txt"),
                    PathBuf::from("/path/to/directory"),
                ]
            );
        }

        #[test]
        fn test_mv_parse_paths_without_strip_slashes() {
            let files = vec![
                OsString::from("/path/to/file1.txt"),
                OsString::from("/path/to/file2.txt"),
                OsString::from("/path/to/directory/"),
            ];
            let mv_options = create_test_opts(MvOverwriteMode::Interactive, false);

            let result = mv_parse_paths(&files, &mv_options);
            assert_eq!(
                result,
                vec![
                    PathBuf::from("/path/to/file1.txt"),
                    PathBuf::from("/path/to/file2.txt"),
                    PathBuf::from("/path/to/directory/"),
                ]
            );
        }
    }
}