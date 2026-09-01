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
/// mv 是 GNU 工具集中的一个命令，用于在类 Unix 系统（如 Linux 和 macOS）中移动文件和目录，或者重命名它们。
mod error;

use crate::opt_flags::ARG_FILES;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use crate::opt_flags::OPT_CONTEXT;
use crate::opt_flags::OPT_DEBUG;
use crate::opt_flags::OPT_FORCE;
use crate::opt_flags::OPT_INTERACTIVE;
use crate::opt_flags::OPT_NO_CLOBBER;
use crate::opt_flags::OPT_NO_COPY;
use crate::opt_flags::OPT_NO_TARGET_DIRECTORY;
use crate::opt_flags::OPT_PROGRESS;
use crate::opt_flags::OPT_STRIP_TRAILING_SLASHES;
use crate::opt_flags::OPT_TARGET_DIRECTORY;
use crate::opt_flags::OPT_VERBOSE;
use clap::builder::ValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_backup_control::{self, source_is_target_backup};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{
    CTError, CTResult, CTsageError, CtSimpleError, FromIo, set_ct_exit_code, strip_errno,
};
use ctcore::ct_fs::{
    are_hardlinks_or_one_way_symlink_to_same_file, are_hardlinks_to_same_file,
    path_ends_with_terminator,
};
#[cfg(target_os = "linux")]
use ctcore::ct_fsxattr;
use ctcore::ct_update_control;
use ctcore::libc;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix;
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows;
use std::path::{Path, PathBuf};
use sys_locale::get_locale;

// 这些枚举（enums）被暴露出来是为了让其他项目（例如 nushell）能够创建一个 Options 值，这需要这些枚举。
pub use ctcore::{ct_backup_control::CtBackupMode, ct_update_control::CtUpdateMode};
use ctcore::{ct_prompt_yes, ct_show};

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

    /// 是否设置目标文件的 SELinux 安全上下文为默认类型
    /// '-Z, --context'
    pub set_context: bool,

    /// 启用调试模式，解释文件是如何被复制的，同时隐含启用 -v (详细) 选项
    /// '--debug'
    pub debug: bool,

    /// 如果重命名失败，不执行复制操作
    /// '--no-copy'
    pub no_copy: bool,
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
    /// 默认模式：仅当目标无写权限且标准输入为 TTY 时提示。
    Default,
}

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
    pub const OPT_CONTEXT: &str = "context";
    pub const OPT_DEBUG: &str = "debug";
    pub const OPT_NO_COPY: &str = "no-copy";
}

pub fn mv_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut command = ct_app();
    let args_match = command.try_get_matches_from_mut(args)?;

    let arg_files: Vec<OsString> = args_match
        .get_many::<OsString>(ARG_FILES)
        .unwrap_or_default()
        .cloned()
        .collect();

    if arg_files.is_empty() {
        return Err(CTsageError::new(1, "missing file operand"));
    }

    if arg_files.len() == 1
        && args_match
            .get_one::<OsString>(OPT_TARGET_DIRECTORY)
            .is_none()
    {
        return Err(CTsageError::new(
            1,
            format!(
                "missing destination file operand after {}",
                arg_files[0].quote()
            ),
        ));
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
        verbose: args_match.get_flag(OPT_VERBOSE) || args_match.get_flag(OPT_DEBUG), // debug implies verbose
        strip_slashes: args_match.get_flag(OPT_STRIP_TRAILING_SLASHES),
        progress_bar: args_match.get_flag(OPT_PROGRESS),
        set_context: args_match.get_flag(OPT_CONTEXT),
        debug: args_match.get_flag(OPT_DEBUG),
        no_copy: args_match.get_flag(OPT_NO_COPY),
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
    let application_info = t!("mv.about");
    let usage_description = t!("mv.usage");
    let after_help = format!(
        "{}\n\n{}",
        t!("mv.after_help"),
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
            .help(t!("mv.clap.opt_force"))
            .overrides_with_all([OPT_INTERACTIVE, OPT_NO_CLOBBER])
            .action(ArgAction::SetTrue),
        Arg::new(OPT_INTERACTIVE)
            .short('i')
            .long(OPT_INTERACTIVE)
            .help(t!("mv.clap.opt_interactive"))
            .overrides_with_all([OPT_FORCE, OPT_NO_CLOBBER])
            .overrides_with(OPT_INTERACTIVE)
            .action(ArgAction::SetTrue),
        Arg::new(OPT_NO_CLOBBER)
            .short('n')
            .long(OPT_NO_CLOBBER)
            .help(t!("mv.clap.opt_no_clobber"))
            .overrides_with_all([OPT_FORCE, OPT_INTERACTIVE])
            .action(ArgAction::SetTrue),
        Arg::new(OPT_STRIP_TRAILING_SLASHES)
            .long(OPT_STRIP_TRAILING_SLASHES)
            .help(t!("mv.clap.opt_strip_trailing_slashes"))
            .action(ArgAction::SetTrue),
        ct_backup_control::arguments::backup(),
        ct_backup_control::arguments::backup_no_args(),
        ct_backup_control::arguments::suffix(),
        ct_update_control::arguments::update(),
        ct_update_control::arguments::update_no_args(),
        Arg::new(OPT_TARGET_DIRECTORY)
            .short('t')
            .long(OPT_TARGET_DIRECTORY)
            .help(t!("mv.clap.opt_target_directory"))
            .value_name("DIRECTORY")
            .value_hint(clap::ValueHint::DirPath)
            .conflicts_with(OPT_NO_TARGET_DIRECTORY)
            .value_parser(ValueParser::os_string()),
        Arg::new(OPT_NO_TARGET_DIRECTORY)
            .short('T')
            .long(OPT_NO_TARGET_DIRECTORY)
            .help(t!("mv.clap.opt_no_target_directory"))
            .action(ArgAction::SetTrue),
        Arg::new(OPT_VERBOSE)
            .short('v')
            .long(OPT_VERBOSE)
            .help(t!("mv.clap.opt_verbose"))
            .action(ArgAction::SetTrue),
        Arg::new(OPT_PROGRESS)
            .short('g')
            .long(OPT_PROGRESS)
            .help(
                "Display a progress bar. \n\
                Note: this feature is not supported by GNU coreutils.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(OPT_CONTEXT)
            .short('Z')
            .long(OPT_CONTEXT)
            .help(t!("mv.clap.opt_context"))
            .action(ArgAction::SetTrue),
        Arg::new(OPT_DEBUG)
            .long(OPT_DEBUG)
            .help(t!("mv.clap.opt_debug"))
            .action(ArgAction::SetTrue),
        Arg::new(OPT_NO_COPY)
            .long(OPT_NO_COPY)
            .help(t!("mv.clap.opt_no_copy"))
            .action(ArgAction::SetTrue),
        Arg::new(ARG_FILES)
            .action(ArgAction::Append)
            .num_args(1..)
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
    if matches.get_flag(OPT_NO_CLOBBER) {
        MvOverwriteMode::NoClobber
    } else if matches.get_flag(OPT_INTERACTIVE) {
        MvOverwriteMode::Interactive
    } else if matches.get_flag(OPT_FORCE) {
        MvOverwriteMode::Force
    } else {
        MvOverwriteMode::Default // 默认遵循 POSIX：对只读文件且在 TTY 时提示
    }
}

/**
 * 在覆盖文件前处理交互提示或强制校验
 */
fn prompt_overwrite(target_path: &Path, mode: &MvOverwriteMode) -> io::Result<()> {
    if *mode == MvOverwriteMode::Force {
        return Ok(());
    }

    let is_interactive = *mode == MvOverwriteMode::Interactive;
    let mut prompt = format!("overwrite {}?", target_path.quote());
    let mut needs_prompt = is_interactive;

    if let Ok(meta) = fs::symlink_metadata(target_path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode_val = meta.permissions().mode() & 0o7777;
            use std::ffi::CString;
            use std::os::unix::ffi::OsStrExt;
            let c_path = CString::new(target_path.as_os_str().as_bytes()).unwrap();
            let is_writable = unsafe { libc::access(c_path.as_ptr(), libc::W_OK) } == 0;

            if !is_writable {
                use std::io::IsTerminal;
                if is_interactive || std::io::stdin().is_terminal() {
                    needs_prompt = true;
                    let rwx = format!(
                        "{}{}{}{}{}{}{}{}{}",
                        if mode_val & 0o400 != 0 { 'r' } else { '-' },
                        if mode_val & 0o200 != 0 { 'w' } else { '-' },
                        if mode_val & 0o100 != 0 {
                            if mode_val & 0o4000 != 0 { 's' } else { 'x' }
                        } else if mode_val & 0o4000 != 0 {
                            'S'
                        } else {
                            '-'
                        },
                        if mode_val & 0o040 != 0 { 'r' } else { '-' },
                        if mode_val & 0o020 != 0 { 'w' } else { '-' },
                        if mode_val & 0o010 != 0 {
                            if mode_val & 0o2000 != 0 { 's' } else { 'x' }
                        } else if mode_val & 0o2000 != 0 {
                            'S'
                        } else {
                            '-'
                        },
                        if mode_val & 0o004 != 0 { 'r' } else { '-' },
                        if mode_val & 0o002 != 0 { 'w' } else { '-' },
                        if mode_val & 0o001 != 0 {
                            if mode_val & 0o1000 != 0 { 't' } else { 'x' }
                        } else if mode_val & 0o1000 != 0 {
                            'T'
                        } else {
                            '-'
                        },
                    );
                    prompt = format!(
                        "replace {}, overriding mode {:04o} ({})?",
                        target_path.quote(),
                        mode_val,
                        rwx
                    );
                }
            }
        }
        #[cfg(not(unix))]
        {
            if meta.permissions().readonly() {
                use std::io::IsTerminal;
                if is_interactive || std::io::stdin().is_terminal() {
                    needs_prompt = true;
                    prompt = format!(
                        "replace {}, overriding mode 0444 (-r--r--r--)?",
                        target_path.quote()
                    );
                }
            }
        }
    }

    if needs_prompt {
        if ct_prompt_yes!("{}", prompt) {
            Ok(())
        } else {
            Err(io::Error::other(""))
        }
    } else {
        Ok(())
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
    mv_files
        .iter()
        .map(|raw| {
            if mv_options.strip_slashes {
                PathBuf::from(strip_trailing_slashes(raw))
            } else {
                PathBuf::from(raw)
            }
        })
        .collect()
}

fn strip_trailing_slashes(path: &OsStr) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let bytes = path.as_bytes();
        if bytes.is_empty() {
            return OsString::new();
        }

        if bytes.iter().all(|b| *b == b'/') {
            // Collapse multiple leading slashes to a single slash
            return OsString::from_vec(vec![b'/']);
        }

        let mut end = bytes.len();
        while end > 0 && bytes[end - 1] == b'/' {
            end -= 1;
        }
        if end == bytes.len() {
            return path.to_os_string();
        }
        OsString::from_vec(bytes[..end].to_vec())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let wide: Vec<u16> = path.encode_wide().collect();
        if wide.is_empty() {
            return OsString::new();
        }

        let is_sep = |unit: u16| unit == b'/' as u16 || unit == b'\\' as u16;
        if wide.iter().all(|unit| is_sep(*unit)) {
            // Preserve a single separator (default to backslash for Windows-style paths)
            let first = if wide.iter().any(|&u| u == b'/' as u16) {
                b'/'
            } else {
                b'\\'
            } as u16;
            return OsString::from_wide(&[first]);
        }

        let mut end = wide.len();
        while end > 0 && is_sep(wide[end - 1]) {
            end -= 1;
        }
        if end == wide.len() {
            return path.to_os_string();
        }
        OsString::from_wide(&wide[..end])
    }
    #[cfg(not(any(unix, windows)))]
    {
        path.to_os_string()
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
    let source_metadata = match source_path.symlink_metadata() {
        Ok(meta) => meta,
        Err(_) => {
            return Err(if path_ends_with_terminator(source_path) {
                MvError::CannotStatNotADirectory(source_path.quote().to_string()).into()
            } else {
                MvError::NoSuchFile(source_path.quote().to_string()).into()
            });
        }
    };
    let target_metadata = target_path.symlink_metadata().ok();
    let source_is_directory = source_metadata.file_type().is_dir();
    let target_is_directory = target_metadata
        .as_ref()
        .is_some_and(|meta| meta.file_type().is_dir());
    let target_exists = target_metadata.is_some();

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
            if source_is_directory {
                match mv_rename(source_path, target_path, mv_options, None, None) {
                    Err(e) => {
                        let err_str = e.to_string();
                        let msg = format!(
                            "cannot move {} to {}",
                            source_path.quote(),
                            target_path.quote()
                        );
                        if err_str.contains("inter-device move failed") || err_str.contains(&msg) {
                            // 处理空的错误（被跳过）
                            if err_str.is_empty() {
                                set_ct_exit_code(1);
                                Ok(())
                            } else {
                                Err(CtSimpleError::new(1, err_str))
                            }
                        } else if err_str.is_empty() {
                            set_ct_exit_code(1);
                            Ok(())
                        } else {
                            Err(e.map_err_context(|| msg))
                        }
                    }
                    Ok(()) => Ok(()),
                }
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
            move_files_into_dir(&[source_path.to_path_buf()], target_path, mv_options)
        }
        // 如果目标存在且源是目录
    } else if target_exists && source_is_directory {
        if mv_options.overwrite == MvOverwriteMode::NoClobber {
            return Ok(());
        }

        // 调用统一的 overwrite 提示校验器
        if let Err(e) = prompt_overwrite(target_path, &mv_options.overwrite) {
            // 遇到空错误不再吞噬，而是设置退出码为 1 并安静返回
            if e.to_string().is_empty() {
                set_ct_exit_code(1);
                return Ok(());
            } else {
                return Err(e.into());
            }
        }
        Err(MvError::NonDirectoryToDirectory(
            source_path.quote().to_string(),
            target_path.quote().to_string(),
        )
        .into())
        // 默认情况：尝试重命名或移动文件。
    } else {
        match mv_rename(source_path, target_path, mv_options, None, None) {
            Ok(()) => Ok(()),
            Err(e) if e.to_string().is_empty() => {
                // 捕获拒绝覆盖传上来的空错误，设置退出状态 1
                set_ct_exit_code(1);
                Ok(())
            }
            Err(e) => Err(CtSimpleError::new(1, format!("{e}"))),
        }
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DirectorySourceKey {
    dev: u64,
    ino: u64,
}

impl DirectorySourceKey {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }
}

fn is_duplicate_directory_source(
    processed_directories: &mut HashMap<DirectorySourceKey, OsString>,
    source_metadata: &fs::Metadata,
    source_path: &Path,
    target_name: &OsStr,
) -> bool {
    let directory_key = DirectorySourceKey::from_metadata(source_metadata);

    if let Some(previous_target_name) = processed_directories.get(&directory_key) {
        if previous_target_name == target_name {
            ct_show!(CtSimpleError::new(
                0,
                format!(
                    "warning: source directory '{}' specified more than once",
                    source_path.display()
                ),
            ));
            return true;
        }
        return false;
    }

    processed_directories.insert(directory_key, target_name.to_os_string());
    false
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

    // 记录已处理的目录源实体，用于将 `./b` 和 `b` 这类同一目录识别为重复源。
    let mut processed_directories: HashMap<DirectorySourceKey, OsString> =
        HashMap::with_capacity(mv_files.len());

    // 标记是否发生过错误
    let mut has_error = false;

    // 检查目标路径是否为目录
    if !target_directory.is_dir() {
        return Err(MvError::NotADirectory(target_directory.quote().to_string()).into());
    }

    // 若目标路径自身不是符号链接，则提前缓存其规范路径，用于后续循环中优化判断。
    let canonical_target_dir = target_directory.symlink_metadata().ok().and_then(|meta| {
        if meta.file_type().is_symlink() {
            None
        } else {
            target_directory.canonicalize().ok()
        }
    });

    // 根据选项决定是否创建进度条
    let multi_progress = mv_opts.progress_bar.then(MultiProgress::new);
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

    // 用于跨分区移动时追踪硬链接关系
    #[cfg(unix)]
    let mut hardlink_map: HashMap<(u64, u64), PathBuf> = HashMap::new();

    // 遍历所有要移动的文件
    for source_path in mv_files {
        if let Some(ref pb) = progress {
            pb.set_message(source_path.to_string_lossy().to_string());
        }

        // 首先检查源文件是否存在
        let source_metadata = match source_path.symlink_metadata() {
            Ok(meta) => meta,
            Err(_) => {
                ct_show!(MvError::NoSuchFile(source_path.quote().to_string()));
                set_ct_exit_code(1);
                has_error = true; // 确保标记了错误
                continue;
            }
        };

        // 确定目标路径
        let target_name = match source_path.file_name() {
            Some(name) => name.to_os_string(),
            None => {
                ct_show!(MvError::NoSuchFile(source_path.quote().to_string()));
                has_error = true;
                continue;
            }
        };
        let targetpath = target_directory.join(&target_name);

        let file_type = source_metadata.file_type();
        if file_type.is_dir()
            && !file_type.is_symlink()
            && is_duplicate_directory_source(
                &mut processed_directories,
                &source_metadata,
                source_path,
                &target_name,
            )
        {
            continue;
        }

        // 检查是否已存在相同目标路径的文件，并根据备份选项处理
        if moved_dests.contains(&targetpath) && mv_opts.backup != CtBackupMode::NumberedBackup {
            ct_show!(CtSimpleError::new(
                1,
                format!(
                    "will not overwrite just-created '{}' with '{}'",
                    targetpath.display(),
                    source_path.display()
                ),
            ));
            has_error = true; // 【核心修复2】发生同名目标冲突时，强制将退出状态设为错误码1
            continue;
        }

        // 检查是否尝试将目录移动到自身
        if let Some(canonical_target) = canonical_target_dir.as_ref() {
            if file_type.is_dir() && !file_type.is_symlink() {
                if let Ok(canonical_source) = source_path.canonicalize() {
                    if canonical_target.starts_with(&canonical_source) {
                        ct_show!(CtSimpleError::new(
                            1,
                            format!(
                                "cannot move '{}' to a subdirectory of itself, '{}'",
                                source_path.display(),
                                targetpath.display()
                            )
                        ));
                        has_error = true;
                        continue;
                    }
                }
            }
        }

        // 尝试重命名文件
        #[cfg(unix)]
        {
            match mv_rename_with_hardlink_tracking(
                source_path,
                &targetpath,
                mv_opts,
                multi_progress.as_ref(),
                &mut hardlink_map,
                true,
            ) {
                Err(err) if err.to_string().is_empty() => {
                    set_ct_exit_code(1);
                    has_error = true;
                }
                Err(err) => {
                    let err_str = err.to_string();
                    let msg = format!(
                        "cannot move {} to {}",
                        source_path.quote(),
                        targetpath.quote()
                    );
                    if err_str.contains("inter-device move failed") {
                        let final_err = CtSimpleError::new(1, err_str);
                        match multi_progress {
                            Some(ref pb) => pb.suspend(|| ct_show!(final_err)),
                            None => ct_show!(final_err),
                        };
                    } else {
                        let final_err = if err_str.contains(&msg) {
                            CtSimpleError::new(1, err_str)
                        } else {
                            err.map_err_context(|| msg)
                        };
                        match multi_progress {
                            Some(ref pb) => pb.suspend(|| ct_show!(final_err)),
                            None => ct_show!(final_err),
                        };
                    }
                    has_error = true;
                }
                Ok(()) => (),
            }
        }
        #[cfg(not(unix))]
        {
            match mv_rename(
                source_path,
                &targetpath,
                mv_opts,
                multi_progress.as_ref(),
                None,
            ) {
                Err(err) if err.to_string().is_empty() => {
                    set_ct_exit_code(1);
                    has_error = true;
                }
                Err(err) => {
                    let err_str = err.to_string();
                    let msg = format!(
                        "cannot move {} to {}",
                        source_path.quote(),
                        targetpath.quote()
                    );
                    if err_str.contains("inter-device move failed") {
                        let final_err = CtSimpleError::new(1, err_str);
                        match multi_progress {
                            Some(ref pb) => pb.suspend(|| ct_show!(final_err)),
                            None => ct_show!(final_err),
                        };
                    } else {
                        let final_err = if err_str.contains(&msg) {
                            CtSimpleError::new(1, err_str)
                        } else {
                            err.map_err_context(|| msg)
                        };
                        match multi_progress {
                            Some(ref pb) => pb.suspend(|| ct_show!(final_err)),
                            None => ct_show!(final_err),
                        };
                    }
                    has_error = true;
                }
                Ok(()) => (),
            }
        }

        // 更新进度条
        if let Some(ref pb) = progress {
            pb.inc(1);
        }
        // 将目标路径加入到已移动文件的集合中
        moved_dests.insert(targetpath.clone());
    }

    // 移动全部文件完成后，如果有错误发生，返回错误
    if has_error {
        return Err(CtSimpleError::new(1, ""));
    }
    Ok(())
}

/**
 * 带硬链接追踪的重命名函数（用于 move_files_into_dir）。
 * 在跨设备移动时，会检查源文件是否与其他已移动的文件是硬链接关系，
 * 如果是，则在目标位置创建硬链接而不是复制文件内容。
 *
 * @param from_path 原始路径。
 * @param to_path 目标路径。
 * @param options 移动选项。
 * @param multi_progress 多重进度条。
 * @param hardlink_map 硬链接映射表，key为(device_id, inode)，value为目标路径。
 * @param use_global_hardlink_map 是否使用全局硬链接映射表（用于跨目录追踪）。
 * @return io::Result<()>
 */
#[cfg(unix)]
fn mv_rename_with_hardlink_tracking(
    from_path: &Path,
    to_path: &Path,
    options: &MvOpts,
    multi_progress: Option<&MultiProgress>,
    hardlink_map: &mut HashMap<(u64, u64), PathBuf>,
    use_global_hardlink_map: bool,
) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    // 获取源文件的元数据
    let source_metadata = from_path.symlink_metadata()?;

    // 只有普通文件（非目录、非符号链接）才需要处理硬链接
    if source_metadata.file_type().is_file() {
        let source_inode = source_metadata.ino();
        let source_dev = source_metadata.dev();
        let key = (source_dev, source_inode);

        // 检查这个 inode 是否已经被移动过
        if let Some(existing_dest) = hardlink_map.get(&key) {
            // 检查目标文件系统是否已存在该文件（需要覆盖）
            if to_path.exists() {
                if options.overwrite == MvOverwriteMode::NoClobber {
                    return Ok(());
                }

                prompt_overwrite(to_path, &options.overwrite)?;

                // 处理备份
                if options.backup != CtBackupMode::NoBackup {
                    let clean_to_os = strip_trailing_slashes(to_path.as_os_str());
                    let clean_to_path = Path::new(&clean_to_os);
                    if let Some(backup_path) = ct_backup_control::get_backup_path(
                        options.backup,
                        clean_to_path,
                        &options.suffix,
                    ) {
                        mv_rename_with_fallback(
                            to_path,
                            &backup_path,
                            options,
                            multi_progress,
                            None,
                        )?;
                    }
                }

                // 删除已存在的目标文件，捕获错误并触发跨设备降级失败
                if let Err(e) = fs::remove_file(to_path) {
                    let err_msg = format!(
                        "inter-device move failed: {} to {}; unable to remove target: {}",
                        from_path.quote(),
                        to_path.quote(),
                        strip_errno(&e)
                    );
                    return Err(io::Error::new(e.kind(), err_msg));
                }
            }

            // 创建硬链接而不是复制
            fs::hard_link(existing_dest, to_path)?;
            // 删除源文件
            fs::remove_file(from_path)?;

            // 输出详细信息
            if options.verbose {
                let message = format!("renamed {} -> {}", from_path.quote(), to_path.quote());
                match multi_progress {
                    Some(pb) => pb.suspend(|| println!("{message}")),
                    None => println!("{message}"),
                };
            }

            // 设置 SELinux 上下文（如果启用）
            #[cfg(target_os = "linux")]
            if options.set_context {
                if let Err(e) = set_default_context(to_path) {
                    eprintln!(
                        "warning: failed to set security context for {}: {}",
                        to_path.quote(),
                        e
                    );
                }
            }

            return Ok(());
        }

        // 如果 nlink > 1，说明这是一个硬链接，需要记录到映射表中
        if source_metadata.nlink() > 1 {
            // 先执行正常的移动操作，传入 hardlink_map 用于目录递归
            if use_global_hardlink_map {
                mv_rename(
                    from_path,
                    to_path,
                    options,
                    multi_progress,
                    Some(hardlink_map),
                )?;
            } else {
                mv_rename(from_path, to_path, options, multi_progress, None)?;
            }
            // 记录这个 inode 对应的目标路径
            hardlink_map.insert(key, to_path.to_path_buf());
            return Ok(());
        }
    }

    // 对于目录、符号链接或普通单链接文件，使用正常的移动逻辑
    if use_global_hardlink_map {
        mv_rename(
            from_path,
            to_path,
            options,
            multi_progress,
            Some(hardlink_map),
        )
    } else {
        mv_rename(from_path, to_path, options, multi_progress, None)
    }
}

/**
 * 重命名文件或目录。
 *
 * @param from_path 原始路径。
 * @param to_path 目标路径。
 * @param options 移动选项，包含更新模式、覆盖模式等。
 * @param multi_progress 多重进度条，用于显示进度。
 * @param hardlink_map 可选的硬链接映射表，用于跨目录追踪硬链接关系。
 * @return io::Result<()>，操作成功返回Ok(())，失败返回Err()。
 */
fn mv_rename(
    from_path: &Path,
    to_path: &Path,
    options: &MvOpts,
    multi_progress: Option<&MultiProgress>,
    #[cfg(unix)] hardlink_map: Option<&mut HashMap<(u64, u64), PathBuf>>,
    #[cfg(not(unix))] _hardlink_map: Option<&mut HashMap<(u64, u64), PathBuf>>,
) -> io::Result<()> {
    let mut backup_path = None;

    // 生成剥离了尾部斜杠的干净路径，用于安全地生成备份名和后续的原子重命名
    let clean_to_os = strip_trailing_slashes(to_path.as_os_str());
    let clean_to_path = Path::new(&clean_to_os);

    // 如果目标路径已存在，根据更新和覆盖选项进行处理
    if to_path.exists() {
        if options.overwrite == MvOverwriteMode::NoClobber {
            return Ok(());
        }

        if options.update == CtUpdateMode::ReplaceNone {
            return Ok(());
        }

        if (options.update == CtUpdateMode::ReplaceIfOlder)
            && fs::metadata(from_path)?.modified()? <= fs::metadata(to_path)?.modified()?
        {
            return Ok(());
        }

        prompt_overwrite(to_path, &options.overwrite)?;

        // 这样 "E/" 就会生成正确的备份名 "E.~1~"，而不是非法的 "E/.~1~"
        backup_path =
            ct_backup_control::get_backup_path(options.backup, clean_to_path, &options.suffix);
        if let Some(ref bp) = backup_path {
            // 将旧的目标文件重命名为备份文件
            mv_rename_with_fallback(to_path, bp, options, multi_progress, None)?;
        }
    }

    // 决定最终用于 rename 系统调用的 target。
    // 如果目标是一个现存的文件，但用户执意输入了 "file/"，我们必须保留斜杠，让操作系统报错 ENOTDIR。
    // 如果目标不存在（或是刚被成功备份移走），我们可以安全地去掉斜杠，实现极速的原子 rename！
    let final_target = if !clean_to_path.exists() || clean_to_path.is_dir() {
        clean_to_path
    } else {
        to_path
    };

    if final_target.exists() && final_target.is_dir() && from_path.is_dir() {
        if is_empty_dir(final_target) {
            fs::remove_dir(final_target)?;
        } else {
            return Err(io::Error::other("Directory not empty"));
        }
    }

    // 执行重命名操作 (使用智能计算出的 final_target)
    mv_rename_with_fallback(
        from_path,
        final_target,
        options,
        multi_progress,
        hardlink_map,
    )?;

    // 如果设置了详细模式，输出重命名信息 (为了符合 GNU 格式，UI 打印必须保留用户最初输入的 to_path)
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

        match multi_progress {
            Some(pb) => pb.suspend(|| {
                println!("{message}");
            }),
            None => println!("{message}"),
        };
    }

    // 如果启用了 context 选项，设置目标文件的 SELinux 上下文
    #[cfg(target_os = "linux")]
    if options.set_context {
        if let Err(e) = set_default_context(final_target) {
            eprintln!(
                "warning: failed to set security context for {}: {}",
                to_path.quote(),
                e
            );
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
pub fn set_default_context(path: &Path) -> io::Result<()> {
    #[cfg(feature = "selinux")]
    {
        // 获取文件的默认安全上下文
        let default_context = match selinux::Context::from_path(path) {
            Ok(ctx) => ctx,
            Err(e) => return Err(io::Error::other(e)),
        };

        // 设置文件的安全上下文
        if let Err(e) = selinux::set_context(path, &default_context) {
            return Err(io::Error::other(e));
        }

        Ok(())
    }

    #[cfg(not(feature = "selinux"))]
    {
        // 当未启用 selinux feature 时，提供警告信息
        eprintln!(
            "warning: failed to set security context for {}: SELinux support not enabled",
            path.quote()
        );
        Ok(())
    }
}
/// 尝试使用 `fs::rename` 更改文件或目录名称，如果失败，则尝试通过复制和删除来备份。
///
/// # 参数
/// - `from`: 指定原始路径。
/// - `to`: 指定目标路径。
/// - `options`: 移动操作的选项，包括调试和禁止复制选项。
/// - `multi_progress`: 可选，用于多进度条更新的 `MultiProgress` 实例，可用于显示复制进度。
/// - `hardlink_map`: 可选的硬链接映射表，用于跨目录追踪硬链接关系。
///
/// # 返回值
/// 返回一个 `io::Result<()>`, 成功则为 `Ok(())`, 失败则为包含错误信息的 `Err`。
fn mv_rename_with_fallback(
    from: &Path,
    to: &Path,
    options: &MvOpts,
    multi_progress: Option<&MultiProgress>,
    #[cfg(unix)] hardlink_map: Option<&mut HashMap<(u64, u64), PathBuf>>,
    #[cfg(not(unix))] _hardlink_map: Option<&mut HashMap<(u64, u64), PathBuf>>,
) -> io::Result<()> {
    // 尝试直接重命名，如果失败则尝试备份方法。
    if let Err(rename_error) = fs::rename(from, to) {
        #[cfg(unix)]
        const EXDEV: i32 = libc::EXDEV as _;
        #[cfg(windows)]
        const EXDEV: i32 = windows_sys::Win32::Foundation::ERROR_NOT_SAME_DEVICE as _;

        let is_cross_device = matches!(rename_error.raw_os_error(), Some(EXDEV));
        // 如果不是跨设备错误，直接返回
        if !is_cross_device {
            let message = format!(
                "cannot move {} to {}: {}",
                from.quote(),
                to.quote(),
                strip_errno(&rename_error)
            );
            return Err(io::Error::new(rename_error.kind(), message));
        }
        // 如果启用了调试模式，说明重命名失败的原因
        if options.debug {
            let message = format!(
                "rename failed: {} ({}), attempting copy and remove",
                from.quote(),
                rename_error
            );
            match multi_progress {
                Some(pb) => pb.suspend(|| {
                    println!("mv: {message}");
                }),
                None => println!("mv: {message}"),
            };
        }

        // 如果启用了 no_copy 选项，在重命名失败时直接返回错误
        if options.no_copy {
            let error_message = if options.debug {
                format!(
                    "rename failed and --no-copy specified: {} to {}",
                    from.quote(),
                    to.quote()
                )
            } else {
                format!("rename failed: {rename_error}")
            };
            return Err(io::Error::other(error_message));
        }

        // 获取原始路径的元数据，不跟随符号链接。
        let symlink_metadata = from.symlink_metadata()?;
        let file_type = symlink_metadata.file_type();

        // 根据文件类型执行相应的备份策略。
        if file_type.is_symlink() {
            // 如果启用了调试模式，说明正在处理符号链接
            if options.debug {
                let message = format!("copying symlink {} to {}", from.quote(), to.quote());
                match multi_progress {
                    Some(pb) => pb.suspend(|| {
                        println!("mv: {message}");
                    }),
                    None => println!("mv: {message}"),
                };
            }
            // 对符号链接执行特定的重命名策略。
            mv_rename_symlink_fallback(from, to)?;
        } else if file_type.is_dir() {
            if rename_error.kind() == io::ErrorKind::NotADirectory {
                let message = format!(
                    "cannot move {} to {}: {}",
                    from.quote(),
                    to.quote(),
                    rename_error
                );
                return Err(io::Error::other(message));
            }

            if options.debug {
                let message = format!("copying directory {} to {}", from.quote(), to.quote());
                match multi_progress {
                    Some(pb) => pb.suspend(|| println!("mv: {message}")),
                    None => println!("mv: {message}"),
                };
            }

            if to.exists() {
                fs::remove_dir_all(to)?;
            }

            // 使用自定义的、带有硬链接记忆表的递归转移函数替代第三方库
            // 如果有外部传入的 hardlink_map，则使用它；否则创建一个新的
            #[cfg(unix)]
            if let Some(map) = hardlink_map {
                // 使用外部传入的映射表，以便跨目录追踪硬链接
                if let Err(e) =
                    move_dir_cross_device_with_links_with_global_map(from, to, options, map)
                {
                    return Err(io::Error::other(format!("{e:?}")));
                }
            } else {
                let mut inode_map = HashMap::new();
                if let Err(e) = move_dir_cross_device_with_links(from, to, options, &mut inode_map)
                {
                    return Err(io::Error::other(format!("{e:?}")));
                }
            }
            #[cfg(not(unix))]
            {
                let mut inode_map = HashMap::new();
                if let Err(e) = move_dir_cross_device_with_links(from, to, options, &mut inode_map)
                {
                    return Err(io::Error::other(format!("{e:?}")));
                }
            }
        } else {
            // 如果启用了调试模式，说明正在处理常规文件或特殊文件
            if options.debug {
                let message = format!("copying file/special {} to {}", from.quote(), to.quote());
                match multi_progress {
                    Some(pb) => pb.suspend(|| {
                        println!("mv: {message}");
                    }),
                    None => println!("mv: {message}"),
                };
            }

            // 在复制或重建之前，如果目标已经存在，必须先删除它！
            // 否则 fs::copy 会以 O_TRUNC 覆盖现有文件，违背 mv 替换 Inode 的语义。
            if to.symlink_metadata().is_ok() {
                if let Err(e) = fs::remove_file(to) {
                    let err_msg = format!(
                        "inter-device move failed: {} to {}; unable to remove target: {}",
                        from.quote(),
                        to.quote(),
                        strip_errno(&e)
                    );
                    return Err(io::Error::new(e.kind(), err_msg));
                }
            }

            // 检查是否为特殊文件（FIFO, 设备节点等），如果是则必须重建而不是读取！
            #[cfg(unix)]
            {
                use std::ffi::CString;
                use std::os::unix::ffi::OsStrExt;
                use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

                let is_special = file_type.is_fifo()
                    || file_type.is_block_device()
                    || file_type.is_char_device()
                    || file_type.is_socket();

                if is_special {
                    let mode = symlink_metadata.permissions().mode() as libc::mode_t;
                    let c_to = CString::new(to.as_os_str().as_bytes()).unwrap();

                    unsafe {
                        if file_type.is_fifo() {
                            if libc::mkfifo(c_to.as_ptr(), mode) != 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                        } else if file_type.is_char_device() || file_type.is_block_device() {
                            let rdev = symlink_metadata.rdev();
                            let s_fmt = if file_type.is_char_device() {
                                libc::S_IFCHR
                            } else {
                                libc::S_IFBLK
                            };
                            if libc::mknod(c_to.as_ptr(), mode | s_fmt, rdev) != 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                        } else {
                            // Socket 跨文件系统移动通常不支持或不需要
                            return Err(std::io::Error::other(
                                "cannot move socket across file systems",
                            ));
                        }
                    }

                    // 特殊文件在目标分区重建成功后，删除原分区文件
                    fs::remove_file(from)?;
                    return Ok(());
                }
            }

            // 如果不是特殊文件，执行常规的跨设备复制并删除
            #[cfg(target_os = "linux")]
            fs::copy(from, to)
                .and_then(|_| ct_fsxattr::ct_copy_xattrs(&from, &to))
                .and_then(|_| fs::remove_file(from))?;

            #[cfg(target_os = "windows")]
            fs::copy(from, to).and_then(|_| fs::remove_file(from))?;

            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
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

    if to.symlink_metadata().is_ok() {
        if let Err(e) = fs::remove_file(to) {
            let err_msg = format!(
                "inter-device move failed: {} to {}; unable to remove target: {}",
                from.quote(),
                to.quote(),
                strip_errno(&e)
            );
            return Err(io::Error::new(e.kind(), err_msg));
        }
    }

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

fn move_dir_cross_device_with_links(
    src_dir: &Path,
    dest_dir: &Path,
    options: &MvOpts,
    inode_map: &mut HashMap<u64, PathBuf>,
) -> io::Result<()> {
    // 确保目标目录存在
    if !dest_dir.exists() {
        fs::create_dir_all(dest_dir)?;
    }

    #[cfg(target_os = "linux")]
    let dir_fsxattrs = ct_fsxattr::ct_retrieve_xattrs(src_dir).unwrap_or_default();

    for entry in fs::read_dir(src_dir)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest_dir.join(entry.file_name());

        if options.verbose {
            println!("renamed {} -> {}", src_path.quote(), dest_path.quote());
        }

        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            // 递归处理子目录
            move_dir_cross_device_with_links(&src_path, &dest_path, options, inode_map)?;
        } else if file_type.is_symlink() {
            // 符号链接直接转移
            mv_rename_symlink_fallback(&src_path, &dest_path)?;
        } else {
            // 处理普通文件，并尝试追踪硬链接
            #[cfg(unix)]
            {
                if let Ok(meta) = fs::symlink_metadata(&src_path) {
                    let inode = meta.ino();

                    // 无论当前的 nlink 是多少，必须先查表！
                    // 因为之前移动并删除它的兄弟硬链接时，当前文件的 nlink 可能已经降为 1 了。
                    if let Some(existing_dest) = inode_map.get(&inode) {
                        // 发现这个 inode 之前已经被复制过了，直接在目标分区重建硬链接
                        fs::hard_link(existing_dest, &dest_path)?;
                        // 原文件使命达成，可以删除了
                        fs::remove_file(&src_path)?;
                        continue;
                    } else if meta.nlink() > 1 {
                        // 第一次遇到这个多链接文件，把它存入记忆表
                        inode_map.insert(inode, dest_path.clone());
                    }
                }
            }

            // 执行常规的复制和删除
            #[cfg(target_os = "linux")]
            {
                fs::copy(&src_path, &dest_path)
                    .and_then(|_| ct_fsxattr::ct_copy_xattrs(&src_path, &dest_path))
                    .and_then(|_| fs::remove_file(&src_path))?;
            }
            #[cfg(not(target_os = "linux"))]
            {
                fs::copy(&src_path, &dest_path).and_then(|_| fs::remove_file(&src_path))?;
            }
        }
    }

    // 恢复目录的扩展属性
    #[cfg(target_os = "linux")]
    let _ = ct_fsxattr::ct_apply_xattrs(dest_dir, dir_fsxattrs);

    // 目录内部清空后，删除原始空目录
    fs::remove_dir(src_dir)?;
    Ok(())
}

/// 使用全局硬链接映射表的目录跨设备移动函数。
/// 与 `move_dir_cross_device_with_links` 不同，此函数使用 (device_id, inode) 作为 key，
/// 以便在跨多个目录移动时追踪硬链接关系。
#[cfg(unix)]
fn move_dir_cross_device_with_links_with_global_map(
    src_dir: &Path,
    dest_dir: &Path,
    options: &MvOpts,
    global_hardlink_map: &mut HashMap<(u64, u64), PathBuf>,
) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    // 确保目标目录存在
    if !dest_dir.exists() {
        fs::create_dir_all(dest_dir)?;
    }

    #[cfg(target_os = "linux")]
    let dir_fsxattrs = ct_fsxattr::ct_retrieve_xattrs(src_dir).unwrap_or_default();

    for entry in fs::read_dir(src_dir)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest_dir.join(entry.file_name());

        if options.verbose {
            println!("renamed {} -> {}", src_path.quote(), dest_path.quote());
        }

        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            // 递归处理子目录，继续使用全局映射表
            move_dir_cross_device_with_links_with_global_map(
                &src_path,
                &dest_path,
                options,
                global_hardlink_map,
            )?;
        } else if file_type.is_symlink() {
            // 符号链接直接转移
            mv_rename_symlink_fallback(&src_path, &dest_path)?;
        } else {
            // 处理普通文件，并尝试追踪硬链接
            if let Ok(meta) = fs::symlink_metadata(&src_path) {
                let inode = meta.ino();
                let dev = meta.dev();
                let key = (dev, inode);

                // 先查全局表，检查这个 inode 是否在其他目录中已经被复制过了
                if let Some(existing_dest) = global_hardlink_map.get(&key) {
                    // 发现这个 inode 之前已经被复制过了，直接在目标分区重建硬链接
                    fs::hard_link(existing_dest, &dest_path)?;
                    // 原文件使命达成，可以删除了
                    fs::remove_file(&src_path)?;
                    continue;
                } else if meta.nlink() > 1 {
                    // 第一次遇到这个多链接文件，把它存入全局记忆表
                    global_hardlink_map.insert(key, dest_path.clone());
                }
            }

            // 执行常规的复制和删除
            #[cfg(target_os = "linux")]
            {
                fs::copy(&src_path, &dest_path)
                    .and_then(|_| ct_fsxattr::ct_copy_xattrs(&src_path, &dest_path))
                    .and_then(|_| fs::remove_file(&src_path))?;
            }
            #[cfg(not(target_os = "linux"))]
            {
                fs::copy(&src_path, &dest_path).and_then(|_| fs::remove_file(&src_path))?;
            }
        }
    }

    // 恢复目录的扩展属性
    #[cfg(target_os = "linux")]
    let _ = ct_fsxattr::ct_apply_xattrs(dest_dir, dir_fsxattrs);

    // 目录内部清空后，删除原始空目录
    fs::remove_dir(src_dir)?;
    Ok(())
}

#[derive(Default)]
pub struct Mv;
impl Tool for Mv {
    fn name(&self) -> &'static str {
        "mv"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        mv_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests_tool_implementation {
    use crate::Mv;
    use ctcore::Tool;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Mv;

        // 测试 name 方法
        assert_eq!(tool.name(), "mv");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("mv"));

        // 测试 execute 方法
        let args = vec![OsString::from("mv"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err()); // --help参数通常会返回错误
    }
}

#[cfg(test)]
mod tests_helper_functions {
    use super::*;
    use clap::ArgMatches;
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;

    fn build_matches(args: &[&str]) -> ArgMatches {
        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push(ctcore::ct_util_name());
        argv.extend_from_slice(args);
        ct_app().try_get_matches_from(argv).expect("参数解析失败")
    }

    fn base_opts() -> MvOpts {
        MvOpts {
            overwrite: MvOverwriteMode::Force,
            backup: CtBackupMode::NoBackup,
            suffix: String::new(),
            update: CtUpdateMode::ReplaceAll,
            target_dir: None,
            no_target_dir: false,
            verbose: false,
            strip_slashes: false,
            progress_bar: false,
            set_context: false,
            debug: false,
            no_copy: false,
        }
    }

    #[test]
    fn overwrite_mode_prefers_no_clobber() {
        let matches = build_matches(&["-n", "from", "to"]);
        assert_eq!(
            mv_determine_overwrite_mode(&matches),
            MvOverwriteMode::NoClobber
        );
    }

    #[test]
    fn overwrite_mode_prefers_interactive() {
        let matches = build_matches(&["-i", "from", "to"]);
        assert_eq!(
            mv_determine_overwrite_mode(&matches),
            MvOverwriteMode::Interactive
        );
    }

    #[test]
    fn overwrite_mode_honours_last_flag() {
        let matches = build_matches(&["-f", "-n", "from", "to"]);
        assert_eq!(
            mv_determine_overwrite_mode(&matches),
            MvOverwriteMode::NoClobber
        );

        let matches = build_matches(&["-n", "-i", "from", "to"]);
        assert_eq!(
            mv_determine_overwrite_mode(&matches),
            MvOverwriteMode::Interactive
        );
    }

    #[test]
    fn parse_paths_respects_strip_trailing_slashes() {
        let files = vec![OsString::from("dir//"), OsString::from("file")];
        let mut opts = base_opts();
        opts.strip_slashes = true;
        let parsed = mv_parse_paths(&files, &opts);
        assert_eq!(parsed[0], PathBuf::from("dir"));
        assert_eq!(parsed[1], PathBuf::from("file"));

        opts.strip_slashes = false;
        let parsed = mv_parse_paths(&files, &opts);
        assert_eq!(parsed[0], PathBuf::from("dir//"));
    }

    #[cfg(unix)]
    #[test]
    fn strip_trailing_slashes_unix_variants() {
        assert_eq!(
            strip_trailing_slashes(OsStr::new("path///")),
            OsString::from("path")
        );
        assert_eq!(
            strip_trailing_slashes(OsStr::new("///")),
            OsString::from("/")
        );
        assert_eq!(
            strip_trailing_slashes(OsStr::new("path/inner")),
            OsString::from("path/inner")
        );
    }

    #[cfg(windows)]
    #[test]
    fn strip_trailing_slashes_windows_variants() {
        assert_eq!(
            strip_trailing_slashes(OsStr::new("path///")),
            OsString::from("path")
        );
        assert_eq!(
            strip_trailing_slashes(OsStr::new(r"\\///")),
            OsString::from("/")
        );
        assert_eq!(
            strip_trailing_slashes(OsStr::new(r"dir\sub///")),
            OsString::from(r"dir\sub")
        );
    }
}

#[cfg(unix)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use tempfile::tempdir;

    fn temp_mv_opts() -> MvOpts {
        MvOpts {
            overwrite: MvOverwriteMode::Force,
            backup: CtBackupMode::NoBackup,
            suffix: String::new(),
            update: CtUpdateMode::ReplaceAll,
            target_dir: None,
            no_target_dir: false,
            verbose: false,
            strip_slashes: false,
            progress_bar: false,
            set_context: false,
            debug: false,
            no_copy: false,
        }
    }

    #[test]
    fn test_move_files_into_dir_moves_all_sources() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let file_a = root.join("a.txt");
        let file_b = root.join("b.txt");
        File::create(&file_a).unwrap();
        File::create(&file_b).unwrap();

        let target = root.join("dest");
        fs::create_dir(&target).unwrap();

        let opts = temp_mv_opts();
        move_files_into_dir(&[file_a.clone(), file_b.clone()], &target, &opts).unwrap();

        assert!(!file_a.exists());
        assert!(!file_b.exists());
        assert!(target.join("a.txt").exists());
        assert!(target.join("b.txt").exists());
    }

    #[test]
    fn test_move_files_into_dir_keeps_distinct_symlink_sources() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let root = temp.path();

        let real_file = root.join("real.txt");
        fs::write(&real_file, b"data").unwrap();

        let link_a = root.join("link_a");
        let link_b = root.join("link_b");
        symlink(&real_file, &link_a).unwrap();
        symlink(&real_file, &link_b).unwrap();

        let target = root.join("dest");
        fs::create_dir(&target).unwrap();

        let opts = temp_mv_opts();
        move_files_into_dir(&[link_a.clone(), link_b.clone()], &target, &opts).unwrap();

        assert!(!link_a.exists());
        assert!(!link_b.exists());
        assert!(target.join("link_a").exists());
        assert!(target.join("link_b").exists());
        assert_eq!(fs::read_link(target.join("link_a")).unwrap(), real_file);
        assert_eq!(fs::read_link(target.join("link_b")).unwrap(), real_file);
    }

    #[test]
    fn test_move_files_into_dir_rejects_non_directory_target() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let file_a = root.join("a.txt");
        File::create(&file_a).unwrap();
        let target = root.join("not_dir");
        File::create(&target).unwrap();

        let opts = temp_mv_opts();
        let err = move_files_into_dir(&[file_a.clone()], &target, &opts).unwrap_err();
        assert!(format!("{err}").contains("Not a directory"));
        assert!(file_a.exists());
    }

    #[test]
    fn test_mv_handle_multiple_paths_no_target_dir_error() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let file_a = root.join("a.txt");
        let file_b = root.join("b.txt");
        File::create(&file_a).unwrap();
        File::create(&file_b).unwrap();
        let target = root.join("dest");
        fs::create_dir(&target).unwrap();

        let mut opts = temp_mv_opts();
        opts.no_target_dir = true;

        let paths = vec![file_a.clone(), file_b.clone(), target.clone()];
        let result = mv_handle_multiple_paths(&paths, &opts);
        assert!(result.is_err());
    }

    #[test]
    fn test_mv_handle_multiple_paths_moves_files() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let file_a = root.join("a.txt");
        let file_b = root.join("b.txt");
        File::create(&file_a).unwrap();
        File::create(&file_b).unwrap();
        let target = root.join("dest");
        fs::create_dir(&target).unwrap();

        let opts = temp_mv_opts();
        mv_handle_multiple_paths(&[file_a.clone(), file_b.clone(), target.clone()], &opts).unwrap();

        assert!(target.join("a.txt").exists());
        assert!(target.join("b.txt").exists());
        assert!(!file_a.exists());
        assert!(!file_b.exists());
    }

    #[test]
    fn test_mv_handle_two_paths_same_file_error() {
        let temp = tempdir().unwrap();
        let file_a = temp.path().join("a.txt");
        File::create(&file_a).unwrap();

        let opts = temp_mv_opts();
        let err = mv_handle_two_paths(&file_a, &file_a, &opts).unwrap_err();
        assert!(err.to_string().contains("the same file"));
    }

    #[test]
    fn test_mv_no_clobber_existing_file_silently_skips() {
        use ctcore::ct_error::{get_ct_exit_code, set_ct_exit_code};

        let temp = tempdir().unwrap();
        let source = temp.path().join("source.txt");
        let target = temp.path().join("target.txt");
        fs::write(&source, b"new").unwrap();
        fs::write(&target, b"old").unwrap();
        set_ct_exit_code(0);

        let mut opts = temp_mv_opts();
        opts.overwrite = MvOverwriteMode::NoClobber;

        mv_rename(&source, &target, &opts, None, None).unwrap();

        assert_eq!(fs::read(&source).unwrap(), b"new");
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(get_ct_exit_code(), 0);
        set_ct_exit_code(0);
    }

    #[test]
    fn test_mv_update_older_overwrites_when_source_is_newer() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source.txt");
        let target = temp.path().join("target.txt");
        fs::write(&source, b"new").unwrap();
        fs::write(&target, b"old").unwrap();

        let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        File::open(&target).unwrap().set_modified(old_time).unwrap();

        let mut opts = temp_mv_opts();
        opts.overwrite = MvOverwriteMode::Force;
        opts.update = CtUpdateMode::ReplaceIfOlder;

        mv_rename(&source, &target, &opts, None, None).unwrap();

        assert!(!source.exists());
        assert_eq!(fs::read(&target).unwrap(), b"new");
    }

    #[test]
    fn test_mv_handle_two_paths_self_subdirectory_error() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("dir");
        let subdir = source.join("inner");
        fs::create_dir_all(&subdir).unwrap();
        let target = subdir.join("nested");
        fs::create_dir(&target).unwrap();

        let opts = temp_mv_opts();
        let err = mv_handle_two_paths(&source, &target, &opts).unwrap_err();
        assert!(err.to_string().contains("cannot move"));
    }

    #[test]
    fn test_mv_rename_directory_not_exdev_should_fail() {
        let temp = tempdir().unwrap();
        let src_dir = temp.path().join("src");
        let nested_file = src_dir.join("hello.txt");
        fs::create_dir(&src_dir).unwrap();
        fs::write(&nested_file, b"hello").unwrap();

        let dest_dir = temp.path().join("dest");
        fs::create_dir(&dest_dir).unwrap();
        fs::write(dest_dir.join("old.txt"), b"old").unwrap();

        let opts = temp_mv_opts();
        let result = mv_rename_with_fallback(&src_dir, &dest_dir, &opts, None, None);

        assert!(result.is_err());
        assert!(src_dir.exists());
    }

    #[test]
    fn test_mv_rename_symlink_fallback_moves_link() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let target_file = temp.path().join("target.txt");
        fs::write(&target_file, b"hello").unwrap();
        let link_path = temp.path().join("link");
        symlink(&target_file, &link_path).unwrap();

        let dest_path = temp.path().join("link_dest");
        mv_rename_symlink_fallback(&link_path, &dest_path).unwrap();

        let metadata = dest_path.symlink_metadata().unwrap();
        assert!(metadata.file_type().is_symlink());
        let resolved = fs::read_link(&dest_path).unwrap();
        assert_eq!(resolved, target_file);
    }

    #[test]
    fn test_is_empty_dir_reports_status() {
        let temp = tempdir().unwrap();
        let dir = temp.path();
        assert!(is_empty_dir(dir));
        fs::write(dir.join("file.txt"), b"data").unwrap();
        assert!(!is_empty_dir(dir));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_set_default_context_without_selinux_feature() {
        let temp = tempdir().unwrap();
        let file = temp.path().join("file.txt");
        fs::write(&file, b"content").unwrap();
        assert!(set_default_context(&file).is_ok());
    }

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
            let args = [ctcore::ct_util_name(), "--version"];

            let result = mv_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_mv_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = mv_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_mv_main_missing_file_operand_with_target_dir() {
            let args = [ctcore::ct_util_name(), "--target=."];
            let err = mv_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(err.to_string(), "missing file operand");
            assert!(err.usage());
        }

        #[test]
        fn test_mv_main_missing_destination_operand() {
            let args = [ctcore::ct_util_name(), "no-file"];
            let err = mv_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(
                err.to_string(),
                "missing destination file operand after 'no-file'"
            );
            assert!(err.usage());
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
            let args = [ctcore::ct_util_name(), src_dir, dst_dir, "-f"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "--force"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [
                ctcore::ct_util_name(),
                src_file,
                "test_mv_main_file_to_file",
                "--force",
            ];
            let result = mv_main(args.iter().map(OsString::from));
            let _ = delete_file("test_mv_main_file_to_file");
            assert!(result.is_ok());
        }

        #[test]
        fn test_mv_main_no_clobber_existing_file_exits_success() {
            use ctcore::ct_error::{get_ct_exit_code, set_ct_exit_code};

            let temp_dir = Builder::new().prefix("test_mv_main_n").tempdir().unwrap();
            let source = temp_dir.path().join("a");
            let target = temp_dir.path().join("b");
            fs::write(&source, b"new").unwrap();
            fs::write(&target, b"old").unwrap();
            set_ct_exit_code(0);

            let args = [
                OsString::from(ctcore::ct_util_name()),
                OsString::from("-n"),
                source.as_os_str().to_os_string(),
                target.as_os_str().to_os_string(),
            ];
            let result = mv_main(args.into_iter());

            assert!(result.is_ok());
            assert_eq!(fs::read(&source).unwrap(), b"new");
            assert_eq!(fs::read(&target).unwrap(), b"old");
            assert_eq!(get_ct_exit_code(), 0);
            set_ct_exit_code(0);
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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "--interactive"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "--no-clobber"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [
                ctcore::ct_util_name(),
                src_file,
                dst_dir,
                "--strip-trailing-slashes",
            ];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "--backup=simple"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "-u"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "--suffix=.bak"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "--update=none"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "--update=all"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "--update=older"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [
                ctcore::ct_util_name(),
                src_file,
                dst_dir,
                "--no-target-directory",
            ];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "--verbose"];
            let result = mv_main(args.iter().map(OsString::from));

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
            let args = [ctcore::ct_util_name(), src_file, dst_dir, "--progress"];
            let result = mv_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }
    #[cfg(test)]
    mod tests_mv_app {
        use crate::ct_app;
        use std::ffi::OsString;

        use crate::opt_flags::{
            OPT_DEBUG, OPT_FORCE, OPT_INTERACTIVE, OPT_NO_CLOBBER, OPT_NO_COPY,
            OPT_NO_TARGET_DIRECTORY, OPT_PROGRESS, OPT_STRIP_TRAILING_SLASHES,
            OPT_TARGET_DIRECTORY, OPT_VERBOSE,
        };
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_f() {
            let args = [ctcore::ct_util_name(), "a", "b", "-f"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_FORCE), Some(&true));
        }

        #[test]
        fn test_ct_app_force() {
            let args = [ctcore::ct_util_name(), "a", "b", "--force"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_FORCE), Some(&true));
        }

        #[test]
        fn test_ct_app_i() {
            let args = [ctcore::ct_util_name(), "a", "b", "-i"];
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
            let args = [ctcore::ct_util_name(), "a", "b", "--interactive"];
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
            let args = [ctcore::ct_util_name(), "a", "b", "-n"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_NO_CLOBBER), Some(&true));
        }

        #[test]
        fn test_ct_app_no_clobber() {
            let args = [ctcore::ct_util_name(), "a", "b", "--no-clobber"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_NO_CLOBBER), Some(&true));
        }

        #[test]
        fn test_ct_app_strip_trailing_slashes() {
            let args = [ctcore::ct_util_name(), "a", "b", "--strip-trailing-slashes"];
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
            let args = [ctcore::ct_util_name(), "a", "b", "--backup=simple"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_b_simple() {
            let args = [ctcore::ct_util_name(), "a", "b", "-b", "simple"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_s() {
            let args = [ctcore::ct_util_name(), "a", "b", "-S", ".bak"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_suffix() {
            let args = [ctcore::ct_util_name(), "a", "b", "--suffix=.bak"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_none() {
            let args = [ctcore::ct_util_name(), "a", "b", "--update=none"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_all() {
            let args = [ctcore::ct_util_name(), "a", "b", "--update=all"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_older() {
            let args = [ctcore::ct_util_name(), "a", "b", "--update=older"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_none() {
            let args = [ctcore::ct_util_name(), "a", "b", "-u", "none"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_all() {
            let args = [ctcore::ct_util_name(), "a", "b", "-u", "all"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_older() {
            let args = [ctcore::ct_util_name(), "a", "b", "-u", "older"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_t_directory() {
            let args = [ctcore::ct_util_name(), "a", "b", "-t", "target-directory"];
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
            let args = [
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
            let args = [ctcore::ct_util_name(), "a", "b", "-T", "target-directory"];
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
            let args = [
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
            let args = [ctcore::ct_util_name(), "a", "b", "-v"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_VERBOSE), Some(&true));
        }
        #[test]
        fn test_ct_app_verbose() {
            let args = [ctcore::ct_util_name(), "a", "b", "--verbose"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_VERBOSE), Some(&true));
        }

        #[test]
        fn test_ct_app_g() {
            let args = [ctcore::ct_util_name(), "a", "b", "-g"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_PROGRESS), Some(&true));
        }
        #[test]
        fn test_ct_app_progress() {
            let args = [ctcore::ct_util_name(), "a", "b", "--progress"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_PROGRESS), Some(&true));
        }

        #[test]
        fn test_ct_app_debug() {
            let args = [ctcore::ct_util_name(), "a", "b", "--debug"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_DEBUG), Some(&true));
        }

        #[test]
        fn test_ct_app_no_copy() {
            let args = [ctcore::ct_util_name(), "a", "b", "--no-copy"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(result.unwrap().get_one::<bool>(OPT_NO_COPY), Some(&true));
        }

        #[test]
        fn test_debug_implies_verbose() {
            let args = [ctcore::ct_util_name(), "a", "b", "--debug"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(matches.get_one::<bool>(OPT_DEBUG), Some(&true));
            // The debug option should imply verbose mode when creating MvOpts
        }
    }
    #[cfg(test)]
    mod tests_mv_fun {
        use crate::{
            DirectorySourceKey, MvOpts, MvOverwriteMode, is_duplicate_directory_source, mv,
            mv_parse_paths,
        };
        use ctcore::ct_backup_control::CtBackupMode;
        use ctcore::ct_update_control::CtUpdateMode;

        use std::collections::HashMap;
        use std::ffi::{OsStr, OsString};
        use std::fs;

        use std::path::{Path, PathBuf};
        use tempfile::tempdir;

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
                set_context: false,
                debug: false,
                no_copy: false,
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

        #[cfg(unix)]
        #[test]
        fn test_mv_symlink_with_trailing_slash() {
            use std::os::unix::fs::symlink;

            let temp = tempdir().unwrap();
            let base = temp.path();
            let real_dir = base.join("testdir");
            fs::create_dir(&real_dir).unwrap();

            let source_link = base.join("testdir1");
            symlink(&real_dir, &source_link).unwrap();

            let mut source_operand = source_link.clone().into_os_string();
            source_operand.push("/");
            let dest_path = base.join("testfile2");
            let dest_operand = dest_path.clone().into_os_string();

            let mut opts = create_test_opts(MvOverwriteMode::Force, true);
            opts.strip_slashes = true;

            let args = vec![source_operand, dest_operand.clone()];
            assert!(mv(&args, &opts).is_ok());
            assert!(!source_link.exists());

            let symlink_meta = fs::symlink_metadata(&dest_path).unwrap();
            assert!(symlink_meta.file_type().is_symlink());
            assert_eq!(fs::read_link(&dest_path).unwrap(), real_dir);
        }

        #[test]
        fn test_duplicate_directory_source_tracks_same_directory_entity() {
            let temp = tempdir().unwrap();
            let base = temp.path();
            let dir = base.join("b");
            fs::create_dir(&dir).unwrap();

            let metadata = fs::symlink_metadata(&dir).unwrap();
            let mut processed_directories = HashMap::new();

            assert!(!is_duplicate_directory_source(
                &mut processed_directories,
                &metadata,
                Path::new("./b"),
                OsStr::new("b")
            ));
            assert_eq!(
                processed_directories.get(&DirectorySourceKey::from_metadata(&metadata)),
                Some(&OsString::from("b"))
            );
            assert!(is_duplicate_directory_source(
                &mut processed_directories,
                &metadata,
                Path::new("b"),
                OsStr::new("b")
            ));
        }
    }
}
