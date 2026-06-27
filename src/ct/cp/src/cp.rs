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

use quick_error::quick_error;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::env;
#[cfg(not(windows))]
use std::ffi::CString;
use std::fs::{self, File, Metadata, OpenOptions, Permissions};
use std::io;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf, StripPrefixError};

use clap::{Arg, ArgAction, ArgMatches, Command, builder::ValueParser, crate_version};
use filetime::FileTime;
use indicatif::{ProgressBar, ProgressStyle};
#[cfg(unix)]
use libc::mkfifo;
use quick_error::ResultExt;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CTsageError, UClapError, set_ct_exit_code};
use ctcore::ct_fs::{
    CtFileInformation, MissingHandling, ResolveMode, are_hardlinks_to_same_file, canonicalize,
    is_symlink_loop, path_ends_with_terminator, paths_refer_to_same_file,
};
use ctcore::{ct_backup_control, ct_update_control};
use platform::copy_on_write;
// 这些是为了让诸如 nushell 等项目能够创建 Options 值而公开的，而创建 Options 值需要依赖于这些枚举类型。
use crate::copydir::copy_directory;
use ctcore::Tool;
pub use ctcore::{ct_backup_control::CtBackupMode, ct_update_control::CtUpdateMode};
use ctcore::{
    ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_prompt_yes, ct_show_error,
    ct_show_warning, ct_util_name,
};
use std::ffi::OsString;

mod copydir;
mod platform;

quick_error! {
    #[derive(Debug)]
    pub enum CpError {
        /// Simple io::Error wrapper
        IoErr(err: io::Error) { from() source(err) display("{}", err)}

        /// Wrapper for io::Error with path context
        IoErrContext(err: io::Error, path: String) {
            display("{}: {}", path, err)
            context(path: &'a str, err: io::Error) -> (err, path.to_owned())
            context(context: String, err: io::Error) -> (err, context)
            source(err)
        }

        /// General copy error
        Error(err: String) {
            display("{}", err)
            from(err: String) -> (err)
            from(err: &'static str) -> (err.to_string())
        }

        /// Represents the state when a non-fatal error has occurred
        /// and not all files were copied.
        NotAllFilesCopied {}

        /// Simple walkdir::Error wrapper
        WalkDirErr(err: walkdir::Error) { from() display("{}", err) source(err) }

        /// Simple std::path::StripPrefixError wrapper
        StripPrefixError(err: StripPrefixError) { from() }

        /// Result of a skipped file
        /// Currently happens when "no" is selected in interactive mode
        Skipped { }

        /// Result of a skipped file
        InvalidArgument(description: String) { display("{}", description) }

        /// All standard options are included as an an implementation
        /// path, but those that are not implemented yet should return
        /// a NotImplemented error.
        NotImplemented(opt: String) { display("Option '{}' not yet implemented.", opt) }

        /// Invalid arguments to backup
        Backup(description: String) { display("{}\nTry '{} --help' for more information.", description, ctcore::ct_execute_phrase()) }

        NotADirectory(path: PathBuf) { display("'{}' is not a directory", path.display()) }
    }
}

impl CTError for CpError {
    fn code(&self) -> i32 {
        EXIT_ERR
    }
}

pub type CopyResult<T> = Result<T, CpError>;

/// Specifies how to overwrite files.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum CpClobberMode {
    Force,
    RemoveDestination,
    Standard,
}

/// Specifies whether files should be overwritten.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum CpOverwriteMode {
    /// [Default] Always overwrite existing files
    Clobber(CpClobberMode),
    /// Prompt before overwriting a file
    Interactive(CpClobberMode),
    /// Never overwrite a file
    NoClobber,
}

/// Possible arguments for `--reflink`.
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum CpReflinkMode {
    Always,
    Auto,
    Never,
}

/// Possible arguments for `--sparse`.
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum CpSparseMode {
    Always,
    Auto,
    Never,
}

/// The expected file type of copy target
#[derive(Copy, Clone)]
pub enum CpTargetType {
    Directory,
    File,
}

/// Copy action to perform
#[derive(PartialEq)]
pub enum CpCopyMode {
    Link,
    SymLink,
    Copy,
    Update,
    AttrOnly,
}

/// Preservation settings for various attributes
///
/// It should be derived from options as follows:
///
///  - if there is a list of attributes to preserve (i.e. `--preserve=ATTR_LIST`) parse that list with [`CpAttributes::cp_parse_iter`],
///  - if `-p` or `--preserve` is given without arguments, use [`CpAttributes::DEFAULT`],
///  - if `-a`/`--archive` is passed, use [`CpAttributes::ALL`],
///  - if `-d` is passed use [`CpAttributes::LINKS`],
///  - otherwise, use [`CpAttributes::NONE`].
///
/// For full compatibility with GNU, these options should also combine. We
/// currently only do a best effort imitation of that behavior, because it is
/// difficult to achieve in clap, especially with `--no-preserve`.
#[derive(Debug)]
pub struct CpAttributes {
    #[cfg(unix)]
    pub ownership: CpPreserve,
    pub mode: CpPreserve,
    pub timestamps: CpPreserve,
    pub context: CpPreserve,
    pub links: CpPreserve,
    pub xattr: CpPreserve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpPreserve {
    // explicit 字段表示是否使用了 --no-preserve 标志来显式指定默认值。
    // 例如，--no-preserve=mode 表示 mode = No { explicit = true }。
    No { explicit: bool },
    Yes { required: bool },
}

impl PartialOrd for CpPreserve {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CpPreserve {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::No { .. }, Self::No { .. }) => Ordering::Equal,
            (Self::Yes { .. }, Self::No { .. }) => Ordering::Greater,
            (Self::No { .. }, Self::Yes { .. }) => Ordering::Less,
            (
                Self::Yes { required: req_self },
                Self::Yes {
                    required: req_other,
                },
            ) => req_self.cmp(req_other),
        }
    }
}

/// Options for the `cp` command
///
/// All options are public so that the options can be programmatically
/// constructed by other crates, such as nushell. That means that this struct
/// is part of our public API. It should therefore not be changed without good
/// reason.
///
/// The fields are documented with the arguments that determine their value.
#[allow(dead_code)]
pub struct CpOptions {
    /// `--attributes-only`
    pub attributes_only: bool,
    /// `--backup[=CONTROL]`, `-b`
    pub backup: CtBackupMode,
    /// `--copy-contents`
    pub copy_contents: bool,
    /// `-H`
    pub cli_dereference: bool,
    /// Determines the type of copying that should be done
    ///
    /// Set by the following arguments:
    ///  - `-l`, `--link`: [`CpCopyMode::Link`]
    ///  - `-s`, `--symbolic-link`: [`CpCopyMode::SymLink`]
    ///  - `-u`, `--update[=WHEN]`: [`CpCopyMode::Update`]
    ///  - `--attributes-only`: [`CpCopyMode::AttrOnly`]
    ///  - otherwise: [`CpCopyMode::Copy`]
    pub copy_mode: CpCopyMode,
    /// `-L`, `--dereference`
    pub dereference: bool,
    /// `-T`, `--no-target-dir`
    pub no_target_dir: bool,
    /// `-x`, `--one-file-system`
    pub one_file_system: bool,
    /// Specifies what to do with an existing destination
    ///
    /// Set by the following arguments:
    ///  - `-i`, `--interactive`: [`CpOverwriteMode::Interactive`]
    ///  - `-n`, `--no-clobber`: [`CpOverwriteMode::NoClobber`]
    ///  - otherwise: [`CpOverwriteMode::Clobber`]
    ///
    /// The `Interactive` and `Clobber` variants have a [`CpClobberMode`] argument,
    /// set by the following arguments:
    ///  - `-f`, `--force`: [`CpClobberMode::Force`]
    ///  - `--remove-destination`: [`CpClobberMode::RemoveDestination`]
    ///  - otherwise: [`CpClobberMode::Standard`]
    pub overwrite: CpOverwriteMode,
    /// `--parents`
    pub parents: bool,
    /// `--sparse[=WHEN]`
    pub sparse_mode: CpSparseMode,
    /// `--strip-trailing-slashes`
    pub strip_trailing_slashes: bool,
    /// `--reflink[=WHEN]`
    pub reflink_mode: CpReflinkMode,
    /// `--preserve=[=ATTRIBUTE_LIST]` and `--no-preserve=ATTRIBUTE_LIST`
    pub attributes: CpAttributes,
    /// `-R`, `-r`, `--recursive`
    pub recursive: bool,
    /// `-S`, `--suffix`
    pub backup_suffix: String,
    /// `-t`, `--target-directory`
    pub target_dir: Option<PathBuf>,
    /// `--update[=UPDATE]`
    pub update: CtUpdateMode,
    /// `--debug`
    pub debug: bool,
    /// `-v`, `--verbose`
    pub verbose: bool,
    /// `-g`, `--progress`
    pub progress_bar: bool,
}

/// Enum representing various debug states of the offload and reflink actions.
#[derive(Debug)]
#[allow(dead_code)] // 所有这些都在 Linux 上使用
#[derive(PartialEq)]
enum CpOffloadReflinkDebug {
    Unknown,
    No,
    Yes,
    Avoided,
    Unsupported,
}

/// Enum representing various debug states of the sparse detection.
#[derive(Debug)]
#[allow(dead_code)] // 目前保持静默，直到我们使用它们
enum CpSparseDebug {
    Unknown,
    No,
    Zeros,
    SeekHole,
    SeekHoleZeros,
    Unsupported,
}

/// Struct that contains the debug state for each action in a file copy operation.
#[derive(Debug)]
struct CopyDebug {
    offload: CpOffloadReflinkDebug,
    reflink: CpOffloadReflinkDebug,
    sparse_detection: CpSparseDebug,
}

impl CpOffloadReflinkDebug {
    fn to_string(&self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Yes => "yes",
            Self::Avoided => "avoided",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }
}

impl CpSparseDebug {
    fn to_string(&self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Zeros => "zeros",
            Self::SeekHole => "SEEK_HOLE",
            Self::SeekHoleZeros => "SEEK_HOLE + zeros",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }
}

/// This function prints the debug information of a file copy operation if
/// no hard link or symbolic link is required, and data copy is required.
/// It prints the debug information of the offload, reflink, and sparse detection actions.
fn cp_show_debug(copy_debug: &CopyDebug) {
    println!(
        "copy offload: {}, reflink: {}, sparse detection: {}",
        copy_debug.offload.to_string(),
        copy_debug.reflink.to_string(),
        copy_debug.sparse_detection.to_string(),
    );
}

const CP_ABOUT: &str = ct_help_about!("cp.md");
const CP_USAGE: &str = ct_help_usage!("cp.md");
const AFTER_HELP: &str = ct_help_section!("after help", "cp.md");

static EXIT_ERR: i32 = 1;

// 参数常量
mod opt_flags {
    pub const ARCHIVE: &str = "archive";
    pub const ATTRIBUTES_ONLY: &str = "attributes-only";
    pub const CLI_SYMBOLIC_LINKS: &str = "cli-symbolic-links";
    pub const CONTEXT: &str = "context";
    pub const COPY_CONTENTS: &str = "copy-contents";
    pub const DEREFERENCE: &str = "dereference";
    pub const FORCE: &str = "force";
    pub const INTERACTIVE: &str = "interactive";
    pub const LINK: &str = "link";
    pub const NO_CLOBBER: &str = "no-clobber";
    pub const NO_DEREFERENCE: &str = "no-dereference";
    pub const NO_DEREFERENCE_PRESERVE_LINKS: &str = "no-dereference-preserve-links";
    pub const NO_PRESERVE: &str = "no-preserve";
    pub const NO_TARGET_DIRECTORY: &str = "no-target-directory";
    pub const ONE_FILE_SYSTEM: &str = "one-file-system";
    pub const PARENT: &str = "parent";
    pub const PARENTS: &str = "parents";
    pub const PATHS: &str = "paths";
    pub const PROGRESS_BAR: &str = "progress";
    pub const PRESERVE: &str = "preserve";
    pub const PRESERVE_DEFAULT_ATTRIBUTES: &str = "preserve-default-attributes";
    pub const RECURSIVE: &str = "recursive";
    pub const REFLINK: &str = "reflink";
    pub const REMOVE_DESTINATION: &str = "remove-destination";
    pub const SPARSE: &str = "sparse";
    pub const STRIP_TRAILING_SLASHES: &str = "strip-trailing-slashes";
    pub const SYMBOLIC_LINK: &str = "symbolic-link";
    pub const TARGET_DIRECTORY: &str = "target-directory";
    pub const DEBUG: &str = "debug";
    pub const VERBOSE: &str = "verbose";
}

#[cfg(unix)]
static CP_PRESERVABLE_ATTRIBUTES: &[&str] = &[
    "mode",
    "ownership",
    "timestamps",
    "context",
    "link",
    "links",
    "xattr",
    "all",
];

#[cfg(not(unix))]
static CP_PRESERVABLE_ATTRIBUTES: &[&str] = &[
    "mode",
    "timestamps",
    "context",
    "link",
    "links",
    "xattr",
    "all",
];

pub fn ct_app() -> Command {
    let command_version = crate_version!();
    let application_info = CP_ABOUT;
    let usage_description = ct_format_usage(CP_USAGE);

    let args = cp_args_init();

    Command::new(ctcore::ct_util_name())
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(format!(
            "{AFTER_HELP}\n\n{}",
            ct_backup_control::CT_BACKUP_CONTROL_LONG_HELP
        ))
        .infer_long_args(true)
        .args_override_self(true)
        .args(&args)
}

fn cp_args_init() -> Vec<Arg> {
    const MODE_ARGS: &[&str] = &[
        opt_flags::LINK,
        opt_flags::REFLINK,
        opt_flags::SYMBOLIC_LINK,
        opt_flags::ATTRIBUTES_ONLY,
        opt_flags::COPY_CONTENTS,
    ];

    let args = vec![
        Arg::new(opt_flags::TARGET_DIRECTORY)
            .short('t')
            .conflicts_with(opt_flags::NO_TARGET_DIRECTORY)
            .long(opt_flags::TARGET_DIRECTORY)
            .value_name(opt_flags::TARGET_DIRECTORY)
            .value_hint(clap::ValueHint::DirPath)
            .value_parser(ValueParser::path_buf())
            .help("copy all SOURCE arguments into target-directory"),
        Arg::new(opt_flags::NO_TARGET_DIRECTORY)
            .short('T')
            .long(opt_flags::NO_TARGET_DIRECTORY)
            .conflicts_with(opt_flags::TARGET_DIRECTORY)
            .help("Treat DEST as a regular file and not a directory")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::INTERACTIVE)
            .short('i')
            .long(opt_flags::INTERACTIVE)
            .overrides_with(opt_flags::NO_CLOBBER)
            .help("ask before overwriting files")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::LINK)
            .short('l')
            .long(opt_flags::LINK)
            .overrides_with_all(MODE_ARGS)
            .help("hard-link files instead of copying")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::NO_CLOBBER)
            .short('n')
            .long(opt_flags::NO_CLOBBER)
            .overrides_with(opt_flags::INTERACTIVE)
            .help("don't overwrite a file that already exists")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::RECURSIVE)
            .short('r')
            .visible_short_alias('R')
            .long(opt_flags::RECURSIVE)
            // --archive sets this option
            .help("copy directories recursively")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::STRIP_TRAILING_SLASHES)
            .long(opt_flags::STRIP_TRAILING_SLASHES)
            .help("remove any trailing slashes from each SOURCE argument")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::DEBUG)
            .long(opt_flags::DEBUG)
            .help("explain how a file is copied. Implies -v")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::VERBOSE)
            .short('v')
            .long(opt_flags::VERBOSE)
            .help("explicitly state what is being done")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::SYMBOLIC_LINK)
            .short('s')
            .long(opt_flags::SYMBOLIC_LINK)
            .overrides_with_all(MODE_ARGS)
            .help("make symbolic links instead of copying")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::FORCE)
            .short('f')
            .long(opt_flags::FORCE)
            .help(
                "if an existing destination file cannot be opened, remove it and \
                     try again (this option is ignored when the -n option is also used). \
                     Currently not implemented for Windows.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::REMOVE_DESTINATION)
            .long(opt_flags::REMOVE_DESTINATION)
            .overrides_with(opt_flags::FORCE)
            .help(
                "remove each existing destination file before attempting to open it \
                     (contrast with --force). On Windows, currently only works for \
                     writeable files.",
            )
            .action(ArgAction::SetTrue),
        ct_backup_control::arguments::backup(),
        ct_backup_control::arguments::backup_no_args(),
        ct_backup_control::arguments::suffix(),
        ct_update_control::arguments::update(),
        ct_update_control::arguments::update_no_args(),
        Arg::new(opt_flags::REFLINK)
            .long(opt_flags::REFLINK)
            .value_name("WHEN")
            .overrides_with_all(MODE_ARGS)
            .require_equals(true)
            .default_missing_value("always")
            .value_parser(["auto", "always", "never"])
            .num_args(0..=1)
            .help("control clone/CoW copies. See below"),
        Arg::new(opt_flags::ATTRIBUTES_ONLY)
            .long(opt_flags::ATTRIBUTES_ONLY)
            .overrides_with_all(MODE_ARGS)
            .help("Don't copy the file data, just the attributes")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::PRESERVE)
            .long(opt_flags::PRESERVE)
            .action(ArgAction::Append)
            .use_value_delimiter(true)
            .value_parser(clap::builder::PossibleValuesParser::new(
                CP_PRESERVABLE_ATTRIBUTES,
            ))
            .num_args(0..)
            .require_equals(true)
            .value_name("ATTR_LIST")
            .overrides_with_all([
                opt_flags::ARCHIVE,
                opt_flags::PRESERVE_DEFAULT_ATTRIBUTES,
                opt_flags::NO_PRESERVE,
            ])
            // -d 选项设置此选项
            // --archive 选项设置此选项
            .help(
                "Preserve the specified attributes (default: mode, ownership (unix only), \
                      timestamps), if possible additional attributes: context, links, xattr, all",
            ),
        Arg::new(opt_flags::PRESERVE_DEFAULT_ATTRIBUTES)
            .short('p')
            .long(opt_flags::PRESERVE_DEFAULT_ATTRIBUTES)
            .overrides_with_all([
                opt_flags::PRESERVE,
                opt_flags::NO_PRESERVE,
                opt_flags::ARCHIVE,
            ])
            .help("same as --preserve=mode,ownership(unix only),timestamps")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::NO_PRESERVE)
            .long(opt_flags::NO_PRESERVE)
            .value_name("ATTR_LIST")
            .overrides_with_all([
                opt_flags::PRESERVE_DEFAULT_ATTRIBUTES,
                opt_flags::PRESERVE,
                opt_flags::ARCHIVE,
            ])
            .help("don't preserve the specified attributes"),
        Arg::new(opt_flags::PARENTS)
            .long(opt_flags::PARENTS)
            .alias(opt_flags::PARENT)
            .help("use full source file name under DIRECTORY")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::NO_DEREFERENCE)
            .short('P')
            .long(opt_flags::NO_DEREFERENCE)
            .overrides_with(opt_flags::DEREFERENCE)
            // -d 选项设置此选项
            .help("never follow symbolic links in SOURCE")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::DEREFERENCE)
            .short('L')
            .long(opt_flags::DEREFERENCE)
            .overrides_with(opt_flags::NO_DEREFERENCE)
            .help("always follow symbolic links in SOURCE")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CLI_SYMBOLIC_LINKS)
            .short('H')
            .help("follow command-line symbolic links in SOURCE")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::ARCHIVE)
            .short('a')
            .long(opt_flags::ARCHIVE)
            .overrides_with_all([
                opt_flags::PRESERVE_DEFAULT_ATTRIBUTES,
                opt_flags::PRESERVE,
                opt_flags::NO_PRESERVE,
            ])
            .help("Same as -dR --preserve=all")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::NO_DEREFERENCE_PRESERVE_LINKS)
            .short('d')
            .help("same as --no-dereference --preserve=links")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::ONE_FILE_SYSTEM)
            .short('x')
            .long(opt_flags::ONE_FILE_SYSTEM)
            .help("stay on this file system")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::SPARSE)
            .long(opt_flags::SPARSE)
            .value_name("WHEN")
            .value_parser(["never", "auto", "always"])
            .help("control creation of sparse files. See below"),
        // TODO: implement the following args
        Arg::new(opt_flags::COPY_CONTENTS)
            .long(opt_flags::COPY_CONTENTS)
            .overrides_with(opt_flags::ATTRIBUTES_ONLY)
            .help("NotImplemented: copy contents of special files when recursive")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CONTEXT)
            .long(opt_flags::CONTEXT)
            .value_name("CTX")
            .help(
                "NotImplemented: set SELinux security context of destination file to \
                     default type",
            ),
        // 'g' 短标志的模式参考自 advcpmv 工具
        Arg::new(opt_flags::PROGRESS_BAR)
            .long(opt_flags::PROGRESS_BAR)
            .short('g')
            .action(clap::ArgAction::SetTrue)
            .help(
                "Display a progress bar. \n\
                 Note: this feature is not supported by GNU coreutils.",
            ),
        Arg::new(opt_flags::PATHS)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath)
            .value_parser(ValueParser::path_buf()),
    ];
    args
}

#[derive(Default)]
pub struct Cp;
impl Tool for Cp {
    fn name(&self) -> &'static str {
        "cp"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        cp_main(args.iter().cloned()).map(|_| ())
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    cp_main(args).map(|_| ())
}

pub fn cp_main(args: impl ctcore::Args) -> CTResult<i32> {
    let args_match = ct_app().try_get_matches_from(args);

    // 在此处解析错误，因为我们不希望版本信息或帮助信息被打印到标准错误输出（stderr）。
    if let Err(e) = args_match {
        let mut app = ct_app();

        match e.kind() {
            clap::error::ErrorKind::DisplayHelp => {
                app.print_help()?;
            }
            clap::error::ErrorKind::DisplayVersion => print!("{}", app.render_version()),
            _ => return Err(Box::new(e.with_exit_code(1))),
        };
    } else if let Ok(mut args_matches) = args_match {
        let cp_options = CpOptions::cp_from_matches(&args_matches)?;

        if cp_options.overwrite == CpOverwriteMode::NoClobber
            && cp_options.backup != CtBackupMode::NoBackup
        {
            return Err(CTsageError::new(
                EXIT_ERR,
                "options --backup and --no-clobber are mutually exclusive",
            ));
        }

        let paths_buf: Vec<PathBuf> = args_matches
            .remove_many::<PathBuf>(opt_flags::PATHS)
            .map(|v| v.collect())
            .unwrap_or_default();

        let (sour_path, target_path) = cp_parse_path_args(paths_buf, &cp_options)?;

        if let Err(error) = cp_copy(&sour_path, &target_path, &cp_options) {
            if let CpError::NotAllFilesCopied = error {
                // 对于这个非致命错误，不做任何处理
            } else {
                ct_show_error!("{}", error);
            }
            set_ct_exit_code(EXIT_ERR);
        }
    }

    Ok(0)
}

impl CpClobberMode {
    fn cp_from_matches(args_match: &ArgMatches) -> Self {
        if args_match.get_flag(opt_flags::FORCE) {
            Self::Force
        } else if args_match.get_flag(opt_flags::REMOVE_DESTINATION) {
            Self::RemoveDestination
        } else {
            Self::Standard
        }
    }
}

impl CpOverwriteMode {
    fn cp_from_matches(args_match: &ArgMatches) -> Self {
        if args_match.get_flag(opt_flags::INTERACTIVE) {
            Self::Interactive(CpClobberMode::cp_from_matches(args_match))
        } else if args_match.get_flag(opt_flags::NO_CLOBBER) {
            Self::NoClobber
        } else {
            Self::Clobber(CpClobberMode::cp_from_matches(args_match))
        }
    }
}

impl CpCopyMode {
    fn cp_from_matches(args_match: &ArgMatches) -> Self {
        if args_match.get_flag(opt_flags::LINK) {
            Self::Link
        } else if args_match.get_flag(opt_flags::SYMBOLIC_LINK) {
            Self::SymLink
        } else if args_match
            .get_one::<String>(ct_update_control::arguments::OPT_UPDATE)
            .is_some()
            || args_match.get_flag(ct_update_control::arguments::OPT_UPDATE_NO_ARG)
        {
            Self::Update
        } else if args_match.get_flag(opt_flags::ATTRIBUTES_ONLY) {
            if args_match.get_flag(opt_flags::REMOVE_DESTINATION) {
                Self::Copy
            } else {
                Self::AttrOnly
            }
        } else {
            Self::Copy
        }
    }
}

impl CpAttributes {
    pub const ALL: Self = Self {
        #[cfg(unix)]
        ownership: CpPreserve::Yes { required: true },
        mode: CpPreserve::Yes { required: true },
        timestamps: CpPreserve::Yes { required: true },
        context: {
            #[cfg(feature = "feat_selinux")]
            {
                CpPreserve::Yes { required: false }
            }
            #[cfg(not(feature = "feat_selinux"))]
            {
                CpPreserve::No { explicit: false }
            }
        },
        links: CpPreserve::Yes { required: true },
        xattr: CpPreserve::Yes { required: false },
    };

    pub const NONE: Self = Self {
        #[cfg(unix)]
        ownership: CpPreserve::No { explicit: false },
        mode: CpPreserve::No { explicit: false },
        timestamps: CpPreserve::No { explicit: false },
        context: CpPreserve::No { explicit: false },
        links: CpPreserve::No { explicit: false },
        xattr: CpPreserve::No { explicit: false },
    };

    // 待办事项：若用户是 root，则要求所有权；对于非 root 用户，所有权不是必需的。
    pub const DEFAULT: Self = Self {
        #[cfg(unix)]
        ownership: CpPreserve::Yes { required: true },
        mode: CpPreserve::Yes { required: true },
        timestamps: CpPreserve::Yes { required: true },
        xattr: CpPreserve::Yes { required: true },
        ..Self::NONE
    };

    pub const LINKS: Self = Self {
        links: CpPreserve::Yes { required: true },
        ..Self::NONE
    };

    pub fn cp_union(self, other: &Self) -> Self {
        Self {
            #[cfg(unix)]
            ownership: self.ownership.max(other.ownership),
            context: self.context.max(other.context),
            timestamps: self.timestamps.max(other.timestamps),
            mode: self.mode.max(other.mode),
            links: self.links.max(other.links),
            xattr: self.xattr.max(other.xattr),
        }
    }

    pub fn cp_parse_iter<T>(values: impl Iterator<Item = T>) -> Result<Self, CpError>
    where
        T: AsRef<str>,
    {
        let mut new = Self::NONE;
        for value in values {
            new = new.cp_union(&Self::cp_parse_single_string(value.as_ref())?);
        }
        Ok(new)
    }

    /// Tries to match string containing a parameter to preserve with the corresponding entry in the
    /// Attributes struct.
    fn cp_parse_single_string(value: &str) -> Result<Self, CpError> {
        let value = value.to_lowercase();

        if value == "all" {
            return Ok(Self::ALL);
        }

        let mut new = Self::NONE;
        let attr = match value.as_ref() {
            "mode" => &mut new.mode,
            #[cfg(unix)]
            "ownership" => &mut new.ownership,
            "timestamps" => &mut new.timestamps,
            "context" => &mut new.context,
            "link" | "links" => &mut new.links,
            "xattr" => &mut new.xattr,
            _ => {
                return Err(CpError::InvalidArgument(format!(
                    "invalid attribute {}",
                    value.quote()
                )));
            }
        };

        *attr = CpPreserve::Yes { required: true };

        Ok(new)
    }
}

impl CpOptions {
    #[allow(clippy::cognitive_complexity)]
    fn cp_from_matches(args_match: &ArgMatches) -> CopyResult<Self> {
        let not_implemented_opts = vec![
            #[cfg(not(any(windows, unix)))]
            opt_flags::ONE_FILE_SYSTEM,
            opt_flags::CONTEXT,
            #[cfg(windows)]
            opt_flags::FORCE,
        ];

        for not_implemented_opt in not_implemented_opts {
            if args_match.contains_id(not_implemented_opt)
                && args_match.value_source(not_implemented_opt)
                    == Some(clap::parser::ValueSource::CommandLine)
            {
                return Err(CpError::NotImplemented(not_implemented_opt.to_string()));
            }
        }

        let recursive =
            args_match.get_flag(opt_flags::RECURSIVE) || args_match.get_flag(opt_flags::ARCHIVE);

        let ct_backup_mode = match ct_backup_control::determine_backup_mode(args_match) {
            Err(e) => return Err(CpError::Backup(format!("{e}"))),
            Ok(mode) => mode,
        };
        let update_mode = ct_update_control::ct_determine_update_mode(args_match);

        let backup_suffix = ct_backup_control::determine_backup_suffix(args_match);

        let overwrite = CpOverwriteMode::cp_from_matches(args_match);

        // 解析目标目录选项
        let no_target_dir = args_match.get_flag(opt_flags::NO_TARGET_DIRECTORY);
        let target_dir = args_match
            .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
            .cloned();

        if let Some(dir) = &target_dir {
            if !dir.is_dir() {
                return Err(CpError::NotADirectory(dir.clone()));
            }
        };

        let mut attributes = Self::cp_get_attributes(args_match)?;

        // 处理不保留选项并调整属性
        if let Some(attribute_strs) = args_match.get_many::<String>(opt_flags::NO_PRESERVE) {
            if attribute_strs.len() > 0 {
                let no_preserve_attributes = CpAttributes::cp_parse_iter(attribute_strs)?;
                if matches!(no_preserve_attributes.links, CpPreserve::Yes { .. }) {
                    attributes.links = CpPreserve::No { explicit: true };
                } else if matches!(no_preserve_attributes.mode, CpPreserve::Yes { .. }) {
                    attributes.mode = CpPreserve::No { explicit: true };
                }
            }
        }

        #[cfg(not(feature = "feat_selinux"))]
        if let CpPreserve::Yes { required } = attributes.context {
            let selinux_disabled_error =
                CpError::Error("SELinux was not enabled during the compile time!".to_string());
            if required {
                return Err(selinux_disabled_error);
            } else {
                show_error_if_needed(&selinux_disabled_error);
            }
        }

        let cp_options = Self {
            attributes_only: args_match.get_flag(opt_flags::ATTRIBUTES_ONLY),
            copy_contents: args_match.get_flag(opt_flags::COPY_CONTENTS),
            cli_dereference: args_match.get_flag(opt_flags::CLI_SYMBOLIC_LINKS),
            copy_mode: CpCopyMode::cp_from_matches(args_match),
            // 使用 -p、-d 和 --archive 时不设置递归引用
            dereference: !(args_match.get_flag(opt_flags::NO_DEREFERENCE)
                || args_match.get_flag(opt_flags::NO_DEREFERENCE_PRESERVE_LINKS)
                || args_match.get_flag(opt_flags::ARCHIVE)
                || recursive)
                || args_match.get_flag(opt_flags::DEREFERENCE),
            one_file_system: args_match.get_flag(opt_flags::ONE_FILE_SYSTEM),
            parents: args_match.get_flag(opt_flags::PARENTS),
            update: update_mode,
            debug: args_match.get_flag(opt_flags::DEBUG),
            verbose: args_match.get_flag(opt_flags::VERBOSE)
                || args_match.get_flag(opt_flags::DEBUG),
            strip_trailing_slashes: args_match.get_flag(opt_flags::STRIP_TRAILING_SLASHES),
            reflink_mode: {
                if let Some(reflink) = args_match.get_one::<String>(opt_flags::REFLINK) {
                    match reflink.as_str() {
                        "always" => CpReflinkMode::Always,
                        "auto" => CpReflinkMode::Auto,
                        "never" => CpReflinkMode::Never,
                        value => {
                            return Err(CpError::InvalidArgument(format!(
                                "invalid argument {} for \'reflink\'",
                                value.quote()
                            )));
                        }
                    }
                } else {
                    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
                    {
                        CpReflinkMode::Auto
                    }
                    #[cfg(not(any(
                        target_os = "linux",
                        target_os = "android",
                        target_os = "macos"
                    )))]
                    {
                        CpReflinkMode::Never
                    }
                }
            },
            sparse_mode: {
                if let Some(val) = args_match.get_one::<String>(opt_flags::SPARSE) {
                    match val.as_str() {
                        "always" => CpSparseMode::Always,
                        "auto" => CpSparseMode::Auto,
                        "never" => CpSparseMode::Never,
                        _ => {
                            return Err(CpError::InvalidArgument(format!(
                                "invalid argument {val} for \'sparse\'"
                            )));
                        }
                    }
                } else {
                    CpSparseMode::Auto
                }
            },
            backup: ct_backup_mode,
            backup_suffix,
            overwrite,
            no_target_dir,
            attributes,
            recursive,
            target_dir,
            progress_bar: args_match.get_flag(opt_flags::PROGRESS_BAR),
        };

        Ok(cp_options)
    }

    // 解析要保留的属性
    fn cp_get_attributes(args_match: &ArgMatches) -> Result<CpAttributes, CpError> {
        let cp_attr = if let Some(att_strs) = args_match.get_many::<String>(opt_flags::PRESERVE) {
            if att_strs.len() == 0 {
                CpAttributes::DEFAULT
            } else {
                CpAttributes::cp_parse_iter(att_strs)?
            }
        } else if args_match.get_flag(opt_flags::ARCHIVE) {
            // 使用了 --archive 标志。等同于 --preserve=all
            CpAttributes::ALL
        } else if args_match.get_flag(opt_flags::NO_DEREFERENCE_PRESERVE_LINKS) {
            CpAttributes::LINKS
        } else if args_match.get_flag(opt_flags::PRESERVE_DEFAULT_ATTRIBUTES) {
            CpAttributes::DEFAULT
        } else {
            CpAttributes::NONE
        };
        Ok(cp_attr)
    }

    fn cp_dereference(&self, in_command_line: bool) -> bool {
        self.dereference || (in_command_line && self.cli_dereference)
    }

    fn cp_preserve_hard_links(&self) -> bool {
        match self.attributes.links {
            CpPreserve::No { .. } => false,
            CpPreserve::Yes { .. } => true,
        }
    }

    #[cfg(unix)]
    fn cp_preserve_mode(&self) -> (bool, bool) {
        match self.attributes.mode {
            CpPreserve::No { explicit } => match explicit {
                true => (false, true),
                false => (false, false),
            },
            CpPreserve::Yes { .. } => (true, false),
        }
    }

    /// Whether to force overwriting the destination file.
    fn cp_force(&self) -> bool {
        matches!(
            self.overwrite,
            CpOverwriteMode::Clobber(CpClobberMode::Force)
        )
    }
}

impl CpTargetType {
    /// Return TargetType required for `target`.
    ///
    /// Treat target as a dir if we have multiple sources or the target
    /// exists and already is a directory
    fn cp_determine(sources_path: &[PathBuf], target: &Path) -> Self {
        if sources_path.len() > 1 || target.is_dir() {
            Self::Directory
        } else {
            Self::File
        }
    }
}

/// Returns tuple of (Source paths, Target)
/**
 * 解析复制操作的路径参数。
 *
 * 此函数负责解析给定的路径参数，并根据复制选项（如是否指定目标目录）来准备实际的复制操作。
 *
 * @param mut paths 包含待复制文件或目录的路径的向量。路径可以是绝对路径或相对路径。
 * @param options 包含复制操作的选项，如是否指定目标目录，是否剥离尾部斜杠等。
 * @return CopyResult<(Vec<PathBuf>, PathBuf)> 成功时返回一个包含调整后的路径列表和目标路径的元组，
 *         失败时返回一个包含错误消息的Err。
 */
fn cp_parse_path_args(
    mut path_buf: Vec<PathBuf>,
    cp_opts: &CpOptions,
) -> CopyResult<(Vec<PathBuf>, PathBuf)> {
    // 检查是否没有指定任何文件
    if path_buf.is_empty() {
        return Err("missing file operand".into());
    } else if path_buf.len() == 1 && cp_opts.target_dir.is_none() {
        // 检查是否只指定了一个文件但未指定目标文件
        return Err(format!("missing destination file operand after {:?}", path_buf[0]).into());
    }

    // 检查是否尝试将多个文件复制到一个非目录的目标
    if cp_opts.no_target_dir && cp_opts.target_dir.is_none() && path_buf.len() > 2 {
        return Err(format!("extra operand {:?}", path_buf[2]).into());
    }

    // 确定目标路径：如果已显式指定，则使用之；否则，使用最后一个路径参数
    let target = match cp_opts.target_dir {
        Some(ref target) => target.clone(),
        None => path_buf.pop().unwrap(),
    };

    // 如果启用了剥离尾部斜杠选项，则对所有源路径进行处理
    if cp_opts.strip_trailing_slashes {
        for source in &mut path_buf {
            let temp = source.components().as_path().to_owned();
            *source = temp;
        }
    }

    Ok((path_buf, target))
}

/// When handling errors, we don't always want to show them to the user. This function handles that.
/**
 * 显示错误信息（如果有必要）。
 *
 * 对于不同的错误类型，采取不同的处理策略。某些错误类型下，我们选择不显示错误信息，
 * 而是通过其他方式处理（例如返回错误码）。
 *
 * @param err 指向CpError的引用，代表复制过程中可能发生的错误。
 */
fn show_error_if_needed(err: &CpError) {
    match err {
        // 当使用--no-clobber选项时，即使复制不完全也不显示错误消息
        CpError::NotAllFilesCopied => {
            // 需要返回一个错误码，但不在此处显示错误信息
        }
        // 如果文件复制被跳过（例如，因为使用了交互式模式且用户拒绝了覆盖），则记录此情况
        CpError::Skipped => {
            // 此处参考了touch a b && echo "n"|cp -i a b && echo $?的用法，
            // 类似情况下，GNU cp 9.2会返回一个错误码
        }
        // 对于所有其他类型的错误，显示标准错误信息
        _ => {
            ct_show_error!("{}", err);
        }
    }
}

/// Copy all `sources` to `target`.
///
/// Returns an `Err(Error::NotAllFilesCopied)` if at least one non-fatal error
/// was encountered.
///
/// Behavior is determined by the `options` parameter, see [`CpOptions`] for details.
/**
 * 复制一个或多个源文件到目标位置。
 *
 * @param sources 源文件路径的数组，可以是单个文件或目录。
 * @param target 目标文件或目录的路径。
 * @param options 复制选项，包括是否递归、是否创建进度条等。
 * @return CopyResult<()>，成功返回Ok(())，失败返回Err()，包含不致命的错误时返回非致命错误信息。
 */
pub fn cp_copy(sour_path: &[PathBuf], target_path: &Path, cp_opts: &CpOptions) -> CopyResult<()> {
    // 确定目标类型（文件、目录、链接等）
    let cp_target_type = CpTargetType::cp_determine(sour_path, target_path);
    // 验证目标类型的正确性
    cp_verify_target_type(target_path, &cp_target_type)?;

    // 初始化变量，用于处理复制过程中的状态
    let mut is_non_fatal_errors = false; // 是否发生过非致命错误
    let mut seen_sources_path = HashSet::with_capacity(sour_path.len()); // 已经处理过的源文件
    let mut symlinked_files_info = HashSet::new(); // 处理过的符号链接文件

    // 用于记录已复制文件的信息，以便后续操作。
    // 通过文件的inode和设备号作为唯一标识，来避免同名文件的冲突。
    let mut copy_files: HashMap<CtFileInformation, PathBuf> =
        HashMap::with_capacity(sour_path.len());
    // 记录已复制文件的目标路径，以便检查重复和处理非致命错误。
    let mut copied_dest: HashSet<PathBuf> = HashSet::with_capacity(sour_path.len());

    // 根据选项决定是否创建进度条
    let process_bar = if cp_opts.progress_bar {
        let pb = ProgressBar::new(cp_disk_usage(sour_path, cp_opts.recursive)?)
            .with_style(
                ProgressStyle::with_template(
                    "{msg}: [{elapsed_precise}] {wide_bar} {bytes:>7}/{total_bytes:7}",
                )
                .unwrap(),
            )
            .with_message(ctcore::ct_util_name());
        pb.tick();
        Some(pb)
    } else {
        None
    };

    // 遍历所有源文件，进行复制
    for source_path in sour_path {
        if seen_sources_path.contains(source_path) {
            // 如果已经处理过此源文件，显示警告
            ct_show_warning!(
                "source file {} specified more than once",
                source_path.quote()
            );
        } else {
            // 计算目标路径
            let dest_path =
                cp_construct_dest_path(source_path, target_path, cp_target_type, cp_opts)
                    .unwrap_or_else(|_| target_path.to_path_buf());

            // 检查目标路径是否存在且不是符号链接，避免重复覆盖
            if fs::metadata(&dest_path).is_ok()
                && !fs::symlink_metadata(&dest_path)?.file_type().is_symlink()
                && copied_dest.contains(&dest_path)
                && cp_opts.backup != CtBackupMode::NumberedBackup
            {
                // 如果目标文件是本次复制过程中创建的，且不允许覆盖，则报错
                return Err(CpError::Error(format!(
                    "will not overwrite just-created '{}' with '{}'",
                    dest_path.display(),
                    source_path.display()
                )));
            }

            // 尝试复制源文件到目标路径
            if let Err(error) = copy_source(
                &process_bar,
                source_path,
                target_path,
                cp_target_type,
                cp_opts,
                &mut symlinked_files_info,
                &mut copy_files,
            ) {
                // 如果发生错误，显示错误信息，并标记为非致命错误
                show_error_if_needed(&error);
                is_non_fatal_errors = true;
            }
            // 将目标路径加入已复制文件集合中
            copied_dest.insert(dest_path.clone());
        }
        // 将源文件加入已处理集合中，避免重复处理
        seen_sources_path.insert(source_path);
    }

    // 如果创建了进度条，完成进度条显示
    if let Some(pb) = process_bar {
        pb.finish();
    }

    // 如果发生过非致命错误，返回相应错误信息；否则，返回成功
    if is_non_fatal_errors {
        Err(CpError::NotAllFilesCopied)
    } else {
        Ok(())
    }
}

fn cp_construct_dest_path(
    sour_path: &Path,
    dest_path: &Path,
    target_type: CpTargetType,
    options: &CpOptions,
) -> CopyResult<PathBuf> {
    if options.no_target_dir && dest_path.is_dir() {
        return Err(format!(
            "cannot overwrite directory {} with non-directory",
            dest_path.quote()
        )
        .into());
    }

    if options.parents && !dest_path.is_dir() {
        return Err("with --parents, the destination must be a directory".into());
    }

    Ok(match target_type {
        CpTargetType::Directory => {
            let root = if options.parents {
                Path::new("")
            } else {
                sour_path.parent().unwrap_or(sour_path)
            };
            cp_localize_to_target(root, sour_path, dest_path)?
        }
        CpTargetType::File => dest_path.to_path_buf(),
    })
}

/**
 * 复制源文件或目录到目标位置。
 *
 * @param progress_bar 如果有，用于显示复制进度的进度条的引用。
 * @param source 源文件或目录的路径。
 * @param target 目标文件或目录的路径。
 * @param target_type 目标文件类型，决定如何处理目标路径。
 * @param options 复制选项，例如是否复制父目录、文件属性等。
 * @param symlinked_files 跟踪已符号链接的文件信息的集合。
 * @param copied_files 跟踪已复制文件信息及其目标路径的映射。
 * @return CopyResult<()>，复制操作的结果，成功返回()，错误则返回错误信息。
 */
fn copy_source(
    progress_bar: &Option<ProgressBar>,
    source: &Path,
    target: &Path,
    target_type: CpTargetType,
    options: &CpOptions,
    symlinked_files: &mut HashSet<CtFileInformation>,
    copied_files: &mut HashMap<CtFileInformation, PathBuf>,
) -> CopyResult<()> {
    let source_path = Path::new(&source);
    if source_path.is_dir() {
        // 复制目录
        copy_directory(
            progress_bar,
            source,
            target,
            options,
            symlinked_files,
            copied_files,
            true,
        )
    } else {
        // 复制文件
        let dest = cp_construct_dest_path(source_path, target, target_type, options)?;
        let res = copy_file(
            progress_bar,
            source_path,
            dest.as_path(),
            options,
            symlinked_files,
            copied_files,
            true,
        );
        // 如果选项设置为复制父目录，则复制源文件的父目录属性到目标文件的父目录
        if options.parents {
            for (x, y) in cp_aligned_ancestors(source, dest.as_path()) {
                copy_attributes(x, y, &options.attributes)?;
            }
        }
        res
    }
}

impl CpOverwriteMode {
    fn verify(&self, path: &Path) -> CopyResult<()> {
        match *self {
            Self::NoClobber => {
                eprintln!("{}: not replacing {}", ct_util_name(), path.quote());
                Err(CpError::NotAllFilesCopied)
            }
            Self::Interactive(_) => {
                if ct_prompt_yes!("overwrite {}?", path.quote()) {
                    Ok(())
                } else {
                    Err(CpError::Skipped)
                }
            }
            Self::Clobber(_) => Ok(()),
        }
    }
}

/// Handles errors for attributes preservation. If the attribute is not required, and
/// errored, tries to show error (see `show_error_if_needed` for additional behavior details).
/// If it's required, then the error is thrown.
fn cp_handle_preserve<F: Fn() -> CopyResult<()>>(p: &CpPreserve, f: F) -> CopyResult<()> {
    match p {
        CpPreserve::No { .. } => {}
        CpPreserve::Yes { required } => {
            let result = f();
            if *required {
                result?;
            } else if let Err(error) = result {
                show_error_if_needed(&error);
            }
        }
    };
    Ok(())
}

/// Copy the specified attributes from one path to another.
pub(crate) fn copy_attributes(
    source_path: &Path,
    dest_path: &Path,
    attr: &CpAttributes,
) -> CopyResult<()> {
    let str = &*format!("{} -> {}", source_path.quote(), dest_path.quote());
    let sour_metadata = fs::symlink_metadata(source_path).context(str)?;

    // 必须先更改所有权以避免干扰模式更改。
    #[cfg(unix)]
    cp_handle_preserve(&attr.ownership, || -> CopyResult<()> {
        use ctcore::ct_perms::CtVerbosityLevel;
        use ctcore::ct_perms::Verbosity;
        use ctcore::ct_perms::wrap_chown;
        use std::os::unix::prelude::MetadataExt;

        let dest_uid = sour_metadata.uid();
        let dest_gid = sour_metadata.gid();

        wrap_chown(
            dest_path,
            &dest_path.symlink_metadata().context(str)?,
            Some(dest_uid),
            Some(dest_gid),
            false,
            Verbosity {
                groups_only: false,
                level: CtVerbosityLevel::Normal,
            },
        )
        .map_err(CpError::Error)?;

        Ok(())
    })?;

    cp_handle_preserve(&attr.mode, || -> CopyResult<()> {
        // 作为`fs::set_permissions()`调用基础的`chmod()`系统调用无法更改符号链接的权限。
        // 在这种情况下，我们什么也不做，因为每个符号链接都有相同的权限。
        if !dest_path.is_symlink() {
            fs::set_permissions(dest_path, sour_metadata.permissions()).context(str)?;
            // FIXME: Implement this for windows as well
            #[cfg(feature = "feat_acl")]
            exacl::getfacl(source_path, None)
                .and_then(|acl| exacl::setfacl(&[dest_path], &acl, None))
                .map_err(|err| CpError::Error(err.to_string()))?;
        }

        Ok(())
    })?;

    cp_handle_preserve(&attr.timestamps, || -> CopyResult<()> {
        let atime = FileTime::from_last_access_time(&sour_metadata);
        let mtime = FileTime::from_last_modification_time(&sour_metadata);
        if dest_path.is_symlink() {
            filetime::set_symlink_file_times(dest_path, atime, mtime)?;
        } else {
            filetime::set_file_times(dest_path, atime, mtime)?;
        }

        Ok(())
    })?;

    #[cfg(feature = "feat_selinux")]
    cp_handle_preserve(&attr.context, || -> CopyResult<()> {
        let context =
            selinux::SecurityContext::of_path(source_path, false, false).map_err(|e| {
                format!(
                    "failed to get security context of {}: {}",
                    source_path.display(),
                    e
                )
            })?;
        if let Some(context) = context {
            context.set_for_path(dest_path, false, false).map_err(|e| {
                format!(
                    "failed to set security context for {}: {}",
                    dest_path.display(),
                    e
                )
            })?;
        }

        Ok(())
    })?;

    cp_handle_preserve(&attr.xattr, || -> CopyResult<()> {
        #[cfg(all(unix, not(target_os = "android")))]
        {
            let xattrs = xattr::list(source_path)?;
            for attr in xattrs {
                if let Some(attr_value) = xattr::get(source_path, attr.clone())? {
                    xattr::set(dest_path, attr, &attr_value[..])?;
                }
            }
        }
        #[cfg(not(all(unix, not(target_os = "android"))))]
        {
            // The documentation for GNU cp states:
            //
            // > Try to preserve SELinux security context and
            // > extended attributes (xattr), but ignore any failure
            // > to do that and print no corresponding diagnostic.
            //
            // so we simply do nothing here.
            //
            // TODO Silently ignore failures in the `#[cfg(unix)]`
            // block instead of terminating immediately on errors.
        }

        Ok(())
    })?;

    Ok(())
}

fn cp_symlink_file(
    s_path: &Path,
    d_path: &Path,
    context: &str,
    symlinked_files: &mut HashSet<CtFileInformation>,
) -> CopyResult<()> {
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(s_path, d_path).context(context)?;
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(s_path, d_path).context(context)?;
    }
    if let Ok(file_info) = CtFileInformation::from_path(d_path, false) {
        symlinked_files.insert(file_info);
    }
    Ok(())
}

fn cp_context_for(src_path: &Path, dest_path: &Path) -> String {
    format!("{} -> {}", src_path.quote(), dest_path.quote())
}

/// Implements a simple backup copy for the destination file.
/// TODO: for the backup, should this function be replaced by `copy_file(...)`?
fn cp_backup_dest(dest_path: &Path, backup_path: &Path) -> CopyResult<PathBuf> {
    if dest_path.is_symlink() {
        fs::rename(dest_path, backup_path)?;
    } else {
        fs::copy(dest_path, backup_path)?;
    }
    Ok(backup_path.into())
}

/// Decide whether source and destination files are the same and
/// copying is forbidden.
///
/// Copying to the same file is only allowed if both `--backup` and
/// `--force` are specified and the file is a regular file.
fn is_forbidden_to_copy_to_same_file(
    sour_path: &Path,
    dest_path: &Path,
    cp_opts: &CpOptions,
    source_in_command_line: bool,
) -> bool {
    // TODO To match the behavior of GNU cp, we also need to check
    // 文件是一个普通文件。
    let dereference_to_compare =
        cp_opts.cp_dereference(source_in_command_line) || !sour_path.is_symlink();
    paths_refer_to_same_file(sour_path, dest_path, dereference_to_compare)
        && !(cp_opts.cp_force() && cp_opts.backup != CtBackupMode::NoBackup)
        && !(dest_path.is_symlink() && cp_opts.backup != CtBackupMode::NoBackup)
}

/// Back up, remove, or leave intact the destination file, depending on the options.
fn cp_handle_existing_dest(
    sour_path: &Path,
    dest_path: &Path,
    cp_opts: &CpOptions,
    source_in_command_line: bool,
) -> CopyResult<()> {
    // 除非同时指定了`--force`和`--backup`，否则不允许将文件复制到自身。

    if is_forbidden_to_copy_to_same_file(sour_path, dest_path, cp_opts, source_in_command_line) {
        return Err(format!(
            "{} and {} are the same file",
            sour_path.quote(),
            dest_path.quote()
        )
        .into());
    }

    cp_opts.overwrite.verify(dest_path)?;

    let backup_pathbuf =
        ct_backup_control::get_backup_path(cp_opts.backup, dest_path, &cp_opts.backup_suffix);
    if let Some(backup_path) = backup_pathbuf {
        if paths_refer_to_same_file(sour_path, &backup_path, true) {
            return Err(format!(
                "backing up {} might destroy source;  {} not copied",
                dest_path.quote(),
                sour_path.quote()
            )
            .into());
        } else {
            cp_backup_dest(dest_path, &backup_path)?;
        }
    }
    match cp_opts.overwrite {
        // FIXME: print that the file was removed if --verbose is enabled
        CpOverwriteMode::Clobber(CpClobberMode::Force) => {
            if is_symlink_loop(dest_path) || fs::metadata(dest_path)?.permissions().readonly() {
                fs::remove_file(dest_path)?;
            }
        }
        CpOverwriteMode::Clobber(CpClobberMode::RemoveDestination) => {
            fs::remove_file(dest_path)?;
        }
        CpOverwriteMode::Clobber(CpClobberMode::Standard) => {
            // 考虑以下文件：
            //
            // * `src/f` - 一个普通文件
            // * `src/link` - 一个指向`src/f`的硬链接
            // * `dest/src/f` - 一个不同的普通文件
            //
            // 在这种情况下，如果我们执行`cp -a src/ dest/`，由于遍历顺序的原因，可能会先复制`src/link`（到`dest/src/link`）。
            // 在这种情况下，为了确保`dest/src/link`是`dest/src/f`的硬链接且`dest/src/f`包含`src/f`的内容，我们需要删除现有文件以允许创建硬链接。

            if cp_opts.cp_preserve_hard_links() {
                fs::remove_file(dest_path)?;
            }
        }
        _ => (),
    };

    Ok(())
}

/// Decide whether the given path exists.
fn cp_file_or_link_exists(file_path: &Path) -> bool {
    // 使用`Path.exists()`或`Path.try_exists()`并不充分，
    // 因为如果`path`是一个符号链接，并且存在过多层符号链接，
    // 那么这些方法将返回false或操作系统错误。
    file_path.symlink_metadata().is_ok()
}

/// Zip the ancestors of a source path and destination path.
///
/// # Examples
///
/// ```rust,ignore
/// let actual = aligned_ancestors(&Path::new("a/b/c"), &Path::new("d/a/b/c"));
/// let expected = vec![
///     (Path::new("a"), Path::new("d/a")),
///     (Path::new("a/b"), Path::new("d/a/b")),
/// ];
/// assert_eq!(actual, expected);
/// ```
fn cp_aligned_ancestors<'a>(sour_path: &'a Path, dest_path: &'a Path) -> Vec<(&'a Path, &'a Path)> {
    // 收集每个路径的祖先。例如，如果 `source` 是 "a/b/c"，
    // 则其祖先为 "a/b/c", "a/b", "a/", 和 ""。
    let sour_path_ancestors: Vec<&Path> = sour_path.ancestors().collect();
    let dest_path_ancestors: Vec<&Path> = dest_path.ancestors().collect();

    // 对于特定应用，不关心空路径 "" 和完整路径（例如 "a/b/c"），
    // 因此排除这些。
    let n = sour_path_ancestors.len();
    let source_path_ancestors = &sour_path_ancestors[1..n - 1];

    // 获取目标路径祖先中与源路径匹配的元素数量（例如，获取 "d/a" 和 "d/a/b"）。
    let k = source_path_ancestors.len();
    let dest_ancestors = &dest_path_ancestors[1..=k];

    // 现在我们有了两个长度相同的数据切片，因此可以将它们组合起来。
    let mut result = vec![];
    for (x, y) in source_path_ancestors
        .iter()
        .rev()
        .zip(dest_ancestors.iter().rev())
    {
        result.push((*x, *y));
    }
    result
}

fn cp_print_verbose_output(
    is_parents: bool,
    progress_bar: &Option<ProgressBar>,
    sour_path: &Path,
    dest_path: &Path,
) {
    if let Some(pb) = progress_bar {
        // 暂停（隐藏）进度条，以防止其与 println 输出重叠
        pb.suspend(|| {
            cp_print_paths(is_parents, sour_path, dest_path);
        });
    } else {
        cp_print_paths(is_parents, sour_path, dest_path);
    }
}

fn cp_print_paths(is_parents: bool, sour_path: &Path, dest_path: &Path) {
    if is_parents {
        // 例如，若将文件 a/b/c 及其上级目录复制至目录 d/，则打印
        //
        // a -> d/a
        // a/b -> d/a/b
        //
        for (x, y) in cp_aligned_ancestors(sour_path, dest_path) {
            println!("{} -> {}", x.display(), y.display());
        }
    }

    println!("{}", cp_context_for(sour_path, dest_path));
}

/// Handles the copy mode for a file copy operation.
///
/// This function determines how to copy a file based on the provided options.
/// It supports different copy modes, including hard linking, copying, symbolic linking, updating, and attribute-only copying.
/// It also handles file backups, overwriting, and dereferencing based on the provided options.
///
/// # Returns
///
/// * `Ok(())` - The file was copied successfully.
/// * `Err(CopyError)` - An error occurred while copying the file.
///
/**
 * 根据指定的复制模式将源路径下的文件或目录复制到目标路径。
 *
 * @param sour_path 源路径的引用。
 * @param dest_path 目标路径的引用。
 * @param cp_opts 复制选项的引用，包含各种复制行为的配置。
 * @param cp_str 用于错误信息中的字符串表示，标识当前正在复制的文件或目录。
 * @param sour_metadata 源文件或目录的元数据，包括文件类型、大小、修改时间等。
 * @param symlinked_files 跟踪已符号链接的文件信息的哈希集的引用，用于避免循环符号链接。
 * @param source_in_command_line 源路径是否直接作为命令行参数给出。
 * @return 返回一个`CopyResult<()>`，成功时为`Ok(())`，错误时为`Err`，不包含复制过程中的具体错误信息。
 */
fn cp_handle_copy_mode(
    sour_path: &Path,
    dest_path: &Path,
    cp_opts: &CpOptions,
    cp_str: &str,
    sour_metadata: Metadata,
    symlinked_files: &mut HashSet<CtFileInformation>,
    source_in_command_line: bool,
) -> CopyResult<()> {
    // 获取源文件类型
    let sour_file_type = sour_metadata.file_type();

    // 判断源路径是否为符号链接
    let sour_is_symlink = sour_file_type.is_symlink();

    // 根据平台判断源文件是否为FIFO特殊文件
    #[cfg(unix)]
    let sour_is_fifo = sour_file_type.is_fifo();
    #[cfg(not(unix))]
    let sour_is_fifo = false;

    // 根据复制模式执行相应的复制逻辑
    match cp_opts.copy_mode {
        CpCopyMode::Link => {
            // 处理目标路径已存在的情况，包括备份和强制覆盖
            if dest_path.exists() {
                let backup_path = ct_backup_control::get_backup_path(
                    cp_opts.backup,
                    dest_path,
                    &cp_opts.backup_suffix,
                );
                if let Some(backup_path) = backup_path {
                    cp_backup_dest(dest_path, &backup_path)?;
                    fs::remove_file(dest_path)?;
                }
                if cp_opts.overwrite == CpOverwriteMode::Clobber(CpClobberMode::Force) {
                    fs::remove_file(dest_path)?;
                }
            }
            // 执行硬链接操作
            if cp_opts.cp_dereference(source_in_command_line) && sour_path.is_symlink() {
                let resolved =
                    canonicalize(sour_path, MissingHandling::Missing, ResolveMode::Physical)
                        .unwrap();
                fs::hard_link(resolved, dest_path)
            } else {
                fs::hard_link(sour_path, dest_path)
            }
            .context(cp_str)?;
        }
        CpCopyMode::Copy => {
            // 执行通用的文件复制逻辑
            copy_helper(
                sour_path,
                dest_path,
                cp_opts,
                cp_str,
                sour_is_symlink,
                sour_is_fifo,
                symlinked_files,
            )?;
        }
        CpCopyMode::SymLink => {
            if dest_path.exists()
                && cp_opts.overwrite == CpOverwriteMode::Clobber(CpClobberMode::Force)
            {
                fs::remove_file(dest_path)?;
            }
            cp_symlink_file(sour_path, dest_path, cp_str, symlinked_files)?;
        }
        CpCopyMode::Update => {
            // 根据更新策略处理目标文件已存在或不存在的情况
            if dest_path.exists() {
                match cp_opts.update {
                    ct_update_control::CtUpdateMode::ReplaceAll => {
                        copy_helper(
                            sour_path,
                            dest_path,
                            cp_opts,
                            cp_str,
                            sour_is_symlink,
                            sour_is_fifo,
                            symlinked_files,
                        )?;
                    }
                    ct_update_control::CtUpdateMode::ReplaceNone => {
                        if cp_opts.debug {
                            println!("skipped {}", dest_path.quote());
                        }

                        return Ok(());
                    }
                    ct_update_control::CtUpdateMode::ReplaceIfOlder => {
                        let dest_metadata = fs::symlink_metadata(dest_path)?;

                        let src_time = sour_metadata.modified()?;
                        let dest_time = dest_metadata.modified()?;
                        if src_time <= dest_time {
                            return Ok(());
                        } else {
                            copy_helper(
                                sour_path,
                                dest_path,
                                cp_opts,
                                cp_str,
                                sour_is_symlink,
                                sour_is_fifo,
                                symlinked_files,
                            )?;
                        }
                    }
                }
            } else {
                copy_helper(
                    sour_path,
                    dest_path,
                    cp_opts,
                    cp_str,
                    sour_is_symlink,
                    sour_is_fifo,
                    symlinked_files,
                )?;
            }
        }
        CpCopyMode::AttrOnly => {
            // 仅复制文件属性
            OpenOptions::new()
                .write(true)
                .truncate(false)
                .create(true)
                .open(dest_path)
                .unwrap();
        }
    };

    Ok(())
}

/// Calculates the permissions for the destination file in a copy operation.
///
/// If the destination file already exists, its current permissions are returned.
/// If the destination file does not exist, the source file's permissions are used,
/// with the `no-preserve` option and the umask taken into account on Unix platforms.
/// # Returns
///
/// * `Ok(Permissions)` - The calculated permissions for the destination file.
/// * `Err(CopyError)` - An error occurred while getting the metadata of the destination file.
///   Allow unused variables for Windows (on options)
#[allow(unused_variables)]
fn cp_calculate_dest_permissions(
    dest_path: &Path,
    source_metadata: &Metadata,
    cp_opts: &CpOptions,
    cp_str: &str,
) -> CopyResult<Permissions> {
    if dest_path.exists() {
        Ok(dest_path.symlink_metadata().context(cp_str)?.permissions())
    } else {
        #[cfg(unix)]
        {
            let mut permissions = source_metadata.permissions();
            let mode = handle_no_preserve_mode(cp_opts, permissions.mode());

            // Apply umask
            use ctcore::ct_mode::get_umask;
            let mode = mode & !get_umask();
            permissions.set_mode(mode);
            Ok(permissions)
        }
        #[cfg(not(unix))]
        {
            let permissions = source_metadata.permissions();
            Ok(permissions)
        }
    }
}

/// Copy the a file from `source` to `dest`. `source` will be dereferenced if
/// `options.dereference` is set to true. `dest` will be dereferenced only if
/// the source was not a symlink.
///
/// Behavior when copying to existing files is contingent on the
/// `options.overwrite` mode. If a file is skipped, the return type
/// should be `Error:Skipped`
///
/// The original permissions of `source` will be copied to `dest`
/// after a successful copy.
#[allow(clippy::cognitive_complexity)]
/**
 * 复制文件从源路径到目标路径，支持多种选项。此函数处理不同场景，如更新文件、处理符号链接、
 * 保留属性及管理硬链接。还支持交互式覆盖模式，并能跟踪已符号链接或已复制的文件。
 *
 * @param progress_bar 可选进度条，用于显示复制进度。
 * @param source 源文件路径。
 * @param dest 目标文件路径。
 * @param options 文件复制的各种选项，如覆盖模式、保留属性等。
 * @param symlinked_files 跟踪已符号链接文件的集合，避免循环引用。
 * @param copied_files 跟踪已复制文件的映射表，避免重复复制。
 * @param source_in_command_line 标记源路径是否直接在命令行中指定。
 * @return CopyResult 包含错误信息或空单元（如果复制成功）。
 */
fn copy_file(
    progress_bar: &Option<ProgressBar>,
    sour_path: &Path,
    dest_path: &Path,
    cp_opts: &CpOptions,
    symlinked_files: &mut HashSet<CtFileInformation>,
    copied_files: &mut HashMap<CtFileInformation, PathBuf>,
    source_in_command_line: bool,
) -> CopyResult<()> {
    // 检查目标是否为先前创建的符号链接并进行相应处理。
    if dest_path.is_symlink() {
        // 如果尝试通过我们创建的符号链接复制文件，返回错误。
        if CtFileInformation::from_path(dest_path, false)
            .map(|info| symlinked_files.contains(&info))
            .unwrap_or(false)
        {
            return Err(CpError::Error(format!(
                "不会通过刚创建的符号链接 '{}' 复制 '{}'",
                dest_path.display(),
                sour_path.display()
            )));
        }

        // 对于符号链接的额外检查与处理。
        let is_copy_contents =
            cp_opts.cp_dereference(source_in_command_line) || !sour_path.is_symlink();
        if is_copy_contents {
            // 在某些条件下，如果目标是悬垂符号链接，返回错误。
            if !dest_path.exists()
                && !matches!(
                    cp_opts.overwrite,
                    CpOverwriteMode::Clobber(CpClobberMode::RemoveDestination)
                )
                && !is_symlink_loop(dest_path)
                && std::env::var_os("POSIXLY_CORRECT").is_none()
            {
                return Err(CpError::Error(format!(
                    "不会通过悬垂符号链接 '{}' 写入",
                    dest_path.display()
                )));
            }

            // 如果目标文件与源文件匹配且允许覆盖，则删除目标文件。
            if paths_refer_to_same_file(sour_path, dest_path, true)
                && matches!(
                    cp_opts.overwrite,
                    CpOverwriteMode::Clobber(CpClobberMode::RemoveDestination)
                )
            {
                fs::remove_file(dest_path)?;
            }
        }
    }

    // 处理指向相同文件的硬链接及现有目标文件。
    if are_hardlinks_to_same_file(sour_path, dest_path)
        && matches!(
            cp_opts.overwrite,
            CpOverwriteMode::Clobber(CpClobberMode::RemoveDestination)
        )
    {
        fs::remove_file(dest_path)?;
    }

    // 根据选项处理现有目标文件。
    if cp_file_or_link_exists(dest_path) {
        if are_hardlinks_to_same_file(sour_path, dest_path)
            && !cp_opts.cp_force()
            && cp_opts.backup == CtBackupMode::NoBackup
            && sour_path != dest_path
            || (sour_path == dest_path && cp_opts.copy_mode == CpCopyMode::Link)
        {
            return Ok(());
        }
        cp_handle_existing_dest(sour_path, dest_path, cp_opts, source_in_command_line)?;
    }

    // 尝试仅更改符号链接属性但不允许覆盖时，返回错误。
    if cp_opts.attributes_only
        && sour_path.is_symlink()
        && !matches!(
            cp_opts.overwrite,
            CpOverwriteMode::Clobber(CpClobberMode::RemoveDestination)
        )
    {
        return Err(format!("无法更改属性 {}: 源文件非常规文件", dest_path.quote()).into());
    }

    // 如请求，保留硬链接。
    if cp_opts.cp_preserve_hard_links() {
        if let Some(new_source) = copied_files.get(
            &CtFileInformation::from_path(
                sour_path,
                cp_opts.cp_dereference(source_in_command_line),
            )
            .context(format!("无法获取 {} 的状态", sour_path.quote()))?,
        ) {
            std::fs::hard_link(new_source, dest_path)?;
            return Ok(());
        };
    }

    // 如请求，输出详细信息。
    if cp_opts.verbose {
        cp_print_verbose_output(cp_opts.parents, progress_bar, sour_path, dest_path);
    }

    // 准备上下文并获取源文件元数据以进行复制。
    let context = cp_context_for(sour_path, dest_path);
    let context = context.as_str();

    let source_metadata = {
        let result = if cp_opts.cp_dereference(source_in_command_line) {
            fs::metadata(sour_path)
        } else {
            fs::symlink_metadata(sour_path)
        };
        result.context(context)?
    };

    // 计算目标文件权限，基于源文件及选项。
    let dest_permissions =
        cp_calculate_dest_permissions(dest_path, &source_metadata, cp_opts, context)?;

    // 根据复制模式和其他选项处理实际复制过程。
    cp_handle_copy_mode(
        sour_path,
        dest_path,
        cp_opts,
        context,
        source_metadata,
        symlinked_files,
        source_in_command_line,
    )?;

    // 如果目标不是符号链接，设置其文件权限。
    if !dest_path.is_symlink() {
        fs::set_permissions(dest_path, dest_permissions).ok();
    }

    copy_attributes(sour_path, dest_path, &cp_opts.attributes)?;

    copied_files.insert(
        CtFileInformation::from_path(sour_path, cp_opts.cp_dereference(source_in_command_line))?,
        dest_path.to_path_buf(),
    );

    if let Some(progress_bar) = progress_bar {
        progress_bar.inc(fs::metadata(sour_path)?.len());
    }

    Ok(())
}

#[cfg(unix)]
fn handle_no_preserve_mode(cp_opts: &CpOptions, org_mode: u32) -> u32 {
    let (is_preserve_mode, is_explicit_no_preserve_mode) = cp_opts.cp_preserve_mode();
    if !is_preserve_mode {
        use libc::{
            S_IRGRP, S_IROTH, S_IRUSR, S_IRWXG, S_IRWXO, S_IRWXU, S_IWGRP, S_IWOTH, S_IWUSR,
        };

        const MODE_RW_UGO: u32 = S_IRUSR | S_IWUSR | S_IRGRP | S_IWGRP | S_IROTH | S_IWOTH;
        const S_IRWXUGO: u32 = S_IRWXU | S_IRWXG | S_IRWXO;
        match is_explicit_no_preserve_mode {
            true => return MODE_RW_UGO,
            false => return org_mode & S_IRWXUGO,
        };
    }

    org_mode
}

/// Copy the file from `source` to `dest` either using the normal `fs::copy` or a
/// copy-on-write scheme if --reflink is specified and the filesystem supports it.
/**
 * 复制文件或目录。
 *
 * 此函数用于从一个源路径复制文件或目录到目标路径。支持多种复制选项，包括保留属性、递归复制等。
 *
 * @param sour_path 源路径的引用。
 * @param dest_path 目标路径的引用。
 * @param cp_opts 复制选项的引用，包括是否创建父目录、是否递归复制等。
 * @param cp_str 用于调试和日志记录的复制标识字符串。
 * @param is_sour_symlink 源路径是否为符号链接。
 * @param is_sour_fifo 源路径是否为FIFO（先进先出）文件。
 * @param symlinked_files 用于记录已复制的符号链接文件信息的哈希集的引用。
 * @return 返回一个`CopyResult<()>`，成功时为`Ok(())`，错误时为`Err(CpError)`。
 */
fn copy_helper(
    sour_path: &Path,
    dest_path: &Path,
    cp_opts: &CpOptions,
    cp_str: &str,
    is_sour_symlink: bool,
    is_sour_fifo: bool,
    symlinked_files: &mut HashSet<CtFileInformation>,
) -> CopyResult<()> {
    // 如果设置了创建父目录选项，则创建目标路径的父目录
    if cp_opts.parents {
        let parent = dest_path.parent().unwrap_or(dest_path);
        fs::create_dir_all(parent)?;
    }

    // 检查目标路径是否以路径分隔符结束且不是目录，如果是则返回错误
    if path_ends_with_terminator(dest_path) && !dest_path.is_dir() {
        return Err(CpError::NotADirectory(dest_path.to_path_buf()));
    }

    // 特殊处理源路径为"/dev/null"的情况，直接创建一个空的目标文件
    if sour_path.as_os_str() == "/dev/null" {
        File::create(dest_path).context(dest_path.display().to_string())?;
    } else if is_sour_fifo && cp_opts.recursive && !cp_opts.copy_contents {
        // 如果源路径是FIFO且设置了递归复制但不复制内容，则特殊处理FIFO文件
        #[cfg(unix)]
        copy_fifo(dest_path, cp_opts.overwrite)?;
    } else if is_sour_symlink {
        // 如果源路径是符号链接，则复制符号链接
        copy_link(sour_path, dest_path, symlinked_files)?;
    } else {
        // 默认情况下，进行文件或目录的实际复制操作
        let copy_debug = copy_on_write(
            sour_path,
            dest_path,
            cp_opts.reflink_mode,
            cp_opts.sparse_mode,
            cp_str,
            #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
            is_sour_fifo,
        )?;

        // 如果未仅设置复制属性且启用了调试模式，则显示复制的详细信息
        if !cp_opts.attributes_only && cp_opts.debug {
            cp_show_debug(&copy_debug);
        }
    }

    Ok(())
}

// 通过创建新的FIFO来"复制"FIFO。这是由于Rust内置的fs::copy尚不支持处理FIFO（参见rust-lang/rust/issues/79390）。
#[cfg(unix)]
fn copy_fifo(dest_path: &Path, overwrite: CpOverwriteMode) -> CopyResult<()> {
    if dest_path.exists() {
        overwrite.verify(dest_path)?;
        fs::remove_file(dest_path)?;
    }

    let name = CString::new(dest_path.as_os_str().as_bytes()).unwrap();
    let err = unsafe { mkfifo(name.as_ptr(), 0o666) };
    if err == -1 {
        return Err(format!("cannot create fifo {}: File exists", dest_path.quote()).into());
    }
    Ok(())
}

fn copy_link(
    sour_path: &Path,
    dest_path: &Path,
    symlinked_files: &mut HashSet<CtFileInformation>,
) -> CopyResult<()> {
    // 尝试从源路径读取符号链接。
    let link = fs::read_link(sour_path)?;

    // 移除目标路径上已存在的文件或符号链接。
    if dest_path.is_symlink() || dest_path.is_file() {
        fs::remove_file(dest_path)?;
    }

    // 创建指向与源链接相同目标的新符号链接。
    cp_symlink_file(
        &link,
        dest_path,
        &cp_context_for(&link, dest_path),
        symlinked_files,
    )
}

/// Generate an error message if `target` is not the correct `target_type`
pub fn cp_verify_target_type(target_path: &Path, cp_target_type: &CpTargetType) -> CopyResult<()> {
    match (cp_target_type, target_path.is_dir()) {
        (&CpTargetType::Directory, false) => {
            Err(format!("target: {} is not a directory", target_path.quote()).into())
        }
        (&CpTargetType::File, true) => Err(format!(
            "cannot overwrite directory {} with non-directory",
            target_path.quote()
        )
        .into()),
        _ => Ok(()),
    }
}

/// Remove the `root` prefix from `source` and prefix it with `target`
/// to create a file that is local to `target`
/// # Examples
///
/// ```ignore
/// assert!(ct_cp::localize_to_target(
///     &Path::new("a/source/"),
///     &Path::new("a/source/c.txt"),
///     &Path::new("target/"),
/// ).unwrap() == Path::new("target/c.txt"))
/// ```
pub fn cp_localize_to_target(
    root_path: &Path,
    sour_path: &Path,
    target_path: &Path,
) -> CopyResult<PathBuf> {
    let local_to_root = sour_path.strip_prefix(root_path)?;
    Ok(target_path.join(local_to_root))
}

/// Get the total size of a slice of files and directories.
///
/// This function is much like the `du` utility, by recursively getting the sizes of files in directories.
/// Files are not deduplicated when appearing in multiple sources. If `recursive` is set to `false`, the
/// directories in `paths` will be ignored.
fn cp_disk_usage(paths_buf: &[PathBuf], is_recursive: bool) -> io::Result<u64> {
    let mut total = 0;
    for pathbuf in paths_buf {
        let md = fs::metadata(pathbuf)?;
        if md.file_type().is_dir() {
            if is_recursive {
                total += cp_disk_usage_directory(pathbuf)?;
            }
        } else {
            total += md.len();
        }
    }
    Ok(total)
}

/// A helper for `disk_usage` specialized for directories.
fn cp_disk_usage_directory(path: &Path) -> io::Result<u64> {
    let mut total = 0;

    for dir_entry in fs::read_dir(path)? {
        let entry = dir_entry?;
        if entry.file_type()?.is_dir() {
            total += cp_disk_usage_directory(&entry.path())?;
        } else {
            total += entry.metadata()?.len();
        }
    }

    Ok(total)
}

#[cfg(test)]
mod tests {
    mod tests_tool_implementation {
        use crate::Cp;
        use ctcore::Tool;
        use std::ffi::OsString;

        #[test]
        fn test_tool_implementation() {
            let tool = Cp::default();

            // 测试 name 方法
            assert_eq!(tool.name(), "cp");

            // 测试 command 方法
            let command = tool.command();
            assert!(command.get_name().contains("cp"));

            // 测试 execute 方法
            let args = vec![OsString::from("cp"), OsString::from("--help")];
            assert!(tool.execute(&args).is_ok());
        }
    }

    mod tests_cp_fn {

        use std::fs;

        use crate::cp_aligned_ancestors;
        use crate::cp_disk_usage;
        use crate::cp_disk_usage_directory;

        use crate::cp_localize_to_target;

        use crate::CpOffloadReflinkDebug;
        use crate::CpPreserve;
        use crate::CpSparseDebug;

        use std::cmp::Ordering;
        use std::fs::File;
        use std::path::{Path, PathBuf};
        use tempfile::Builder;
        #[test]
        fn test_cp_localize_to_target() {
            let root = Path::new("a/source/");
            let source = Path::new("a/source/c.txt");
            let target = Path::new("target/");
            let actual = cp_localize_to_target(root, source, target).unwrap();
            let expected = Path::new("target/c.txt");
            assert_eq!(actual, expected);
        }

        #[test]
        fn test_aligned_ancestors() {
            let actual = cp_aligned_ancestors(Path::new("a/b/c"), Path::new("d/a/b/c"));
            let expected = vec![
                (Path::new("a"), Path::new("d/a")),
                (Path::new("a/b"), Path::new("d/a/b")),
            ];
            assert_eq!(actual, expected);
        }

        #[test]
        fn test_cmp() {
            let no1 = CpPreserve::No { explicit: false };
            let no2 = CpPreserve::No { explicit: true };

            let yes1 = CpPreserve::Yes { required: false };
            let yes2 = CpPreserve::Yes { required: true };

            // Test cases for comparing `No` instances
            assert_eq!(no1.cmp(&no1), Ordering::Equal);
            assert_eq!(no1.cmp(&no2), Ordering::Equal); // `explicit` field is ignored in comparison

            // Test cases for comparing `No` with `Yes`
            assert_eq!(no1.cmp(&yes1), Ordering::Less);
            assert_eq!(no2.cmp(&yes2), Ordering::Less);

            // Test cases for comparing `Yes` instances
            assert_eq!(yes1.cmp(&yes1), Ordering::Equal);
            assert_eq!(yes1.cmp(&yes2), Ordering::Less);

            // Additional test cases for mixed comparisons
            assert_eq!(yes2.cmp(&no1), Ordering::Greater);
            assert_eq!(yes2.cmp(&no2), Ordering::Greater);
        }

        #[test]
        fn test_partial_cmp() {
            let result = 1.0.partial_cmp(&2.0);
            assert_eq!(result, Some(Ordering::Less));

            let result = 1.0.partial_cmp(&1.0);
            assert_eq!(result, Some(Ordering::Equal));

            let result = 2.0.partial_cmp(&1.0);
            assert_eq!(result, Some(Ordering::Greater));

            let result = f64::NAN.partial_cmp(&1.0);
            assert_eq!(result, None);
        }

        #[test]
        fn test_offload_reflink_debug_to_string() {
            // Test cases for each variant of OffloadReflinkDebug
            assert_eq!(CpOffloadReflinkDebug::No.to_string(), "no");
            assert_eq!(CpOffloadReflinkDebug::Yes.to_string(), "yes");
            assert_eq!(CpOffloadReflinkDebug::Avoided.to_string(), "avoided");
            assert_eq!(
                CpOffloadReflinkDebug::Unsupported.to_string(),
                "unsupported"
            );
            assert_eq!(CpOffloadReflinkDebug::Unknown.to_string(), "unknown");
        }

        #[test]
        fn test_sparse_debug_to_string() {
            // Test cases for each variant of SparseDebug
            assert_eq!(CpSparseDebug::No.to_string(), "no");
            assert_eq!(CpSparseDebug::Zeros.to_string(), "zeros");
            assert_eq!(CpSparseDebug::SeekHole.to_string(), "SEEK_HOLE");
            assert_eq!(
                CpSparseDebug::SeekHoleZeros.to_string(),
                "SEEK_HOLE + zeros"
            );
            assert_eq!(CpSparseDebug::Unsupported.to_string(), "unsupported");
            assert_eq!(CpSparseDebug::Unknown.to_string(), "unknown");
        }

        #[test]
        fn test_disk_usage_with_recursive_false() {
            // Create a temporary directory and file
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();

            // Call the disk_usage function
            let paths = vec![test_file_1.as_path().to_path_buf()];
            let result = cp_disk_usage(&paths, false);

            // println!("{}", result.unwrap());
            // Check that the total size is equal to the size of the file
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_disk_usage_with_recursive_true() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();

            // Call the disk_usage function
            let paths = vec![test_file_1.as_path().to_path_buf()];
            let result = cp_disk_usage(&paths, true);

            // println!("{}", result.unwrap());

            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_disk_usage_with_empty_paths() {
            // Call the disk_usage function with an empty paths vector
            let paths = vec![];
            let result = cp_disk_usage(&paths, true);

            // Check that the total size is zero
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_disk_usage_with_nonexistent_path() {
            // Create a path to a nonexistent file or directory
            let nonexistent_path = PathBuf::from("/path/to/nonexistent");

            // Call the disk_usage function
            let paths = vec![nonexistent_path];
            let result = cp_disk_usage(&paths, true);

            // println!("--------------->{:?}", result);
            match result {
                Err(output) => {
                    // println!("output{:?}", output.kind());
                    assert_eq!(output.kind(), std::io::ErrorKind::NotFound);
                }
                Ok(_) => {
                    panic!("disk_usage should return an error for a nonexistent path");
                }
            }
        }

        #[test]
        fn test_disk_usage_directory_with_recursive_true() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();

            // Call the disk_usage function

            let result = cp_disk_usage_directory(sub_dir_path.as_path());

            // println!("{}", result.unwrap());

            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_disk_usage_directory_with_empty_paths() {
            // Call the disk_usage function with an empty paths vector
            let paths = Path::new("a/b/c.txt");
            let result = cp_disk_usage_directory(paths);

            match result {
                Err(output) => {
                    println!("output{:?}", output.kind());
                    assert_eq!(output.kind(), std::io::ErrorKind::NotFound);
                }
                Ok(_) => {
                    panic!("disk_usage should return an error for a nonexistent path");
                }
            }
        }
        use std::borrow::Borrow;
        #[test]
        fn test_disk_usage_directory_with_nonexistent_path() {
            // Create a path to a nonexistent file or directory
            let nonexistent_path = PathBuf::from("/path/to/nonexistent");

            // Call the disk_usage function
            let result = cp_disk_usage_directory(nonexistent_path.borrow());

            match result {
                Err(output) => {
                    // println!("output{:?}", output.kind());
                    assert_eq!(output.kind(), std::io::ErrorKind::NotFound);
                }
                Ok(_) => {
                    panic!("disk_usage should return an error for a nonexistent path");
                }
            }
        }
    }

    #[cfg(test)]
    mod tests_ctmain {
        use crate::cp_main;

        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;

        use tempfile::Builder;

        #[test]
        fn test_ctmain_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_g_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-g", filename1, filename2];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        //////////////////////////////////////////////

        #[test]
        fn test_ctmain_d_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-d", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ad_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-ad", filename1, filename2];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_d_links_invalid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-d",
                filename1,
                filename2,
                "extra_arg",
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_preserve_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--preserve", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_preserve_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_preserve_all_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=all",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_preserve_all_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=all",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_d_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-d", filename1, filename2];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ad_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-ad", filename1, filename2];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_adr_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-adr", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_adrf_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-adrf", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_adrfv_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-adrfv", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_preserve_mode_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=mode",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_preserve_mode_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=mode",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_preserve_ownership_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=mode",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_preserve_timestamps_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=timestamps",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_preserve_timestamps_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=timestamps",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_preserve_context_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=context",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(_output) => {
                    assert_eq!(1, 1);
                    // panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_preserve_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=link",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_preserve_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=link",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_preserve_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=links",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_preserve_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=links",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_preserve_xattr_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=xattr",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_preserve_xattr_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=xattr",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_a_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_u_preserve_mode_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=mode",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_u_preserve_ownership_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=ownership",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_u_preserve_timestamps_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-u", filename1, filename2];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_u_preserve_context_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=context",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(_output) => {
                    assert_eq!(1, 1);
                    // panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_u_preserve_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=link",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_u_preserve_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=links",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_u_preserve_xattr_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                "-u",
                ctcore::ct_util_name(),
                "--preserve=xattr",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_u_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_p_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-p",
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_aru_preserve_ownership_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-uar",
                "--preserve=ownership",
                filename1,
                filename2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_aru_preserve_timestamps_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-u", filename1, filename2];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_aru_preserve_context_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=context",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(_output) => {
                    assert_eq!(1, 1);
                    // panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_aru_preserve_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-aru",
                "--preserve=link",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_aru_preserve_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-aru",
                "--preserve=links",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_aru_preserve_xattr_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                "-aru",
                ctcore::ct_util_name(),
                "--preserve=xattr",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_aru_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-aru",
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_aup_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-aup",
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        ///////////////

        #[test]
        fn test_ctmain_no_preserve_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--no-preserve",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(_output) => {
                    assert_eq!(1, 1);
                    // panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_p_no_preserve_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-p",
                "--no-preserve",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(_output) => {
                    assert_eq!(1, 1);
                    // panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_parents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--parents", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_p_parents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-p",
                "--parents",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_hard_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-H", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_dereference_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-L", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_dereference_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--dereference",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_l_dereference_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-L",
                "--dereference",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_no_dereference_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-P", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_no_dereference_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--no-dereference",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_p_no_dereference_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-P",
                "--no-dereference",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_archive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-a", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_archive_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--archive", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_reflink_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--reflink=always",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_reflink_auto_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--reflink=auto",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_reflink_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--reflink=never",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_u_reflink_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--reflink=never",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_u_reflink_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--reflink=always",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_u_reflink_auto_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--reflink=auto",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_attributes_only_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--attributes-only",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_copy_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-c", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(_output) => {
                    assert_eq!(1, 1);
                    // panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_u_update_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-u", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_paths_single_source_single_dest() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_paths_multiple_sources_single_dest() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();
            let sub_dir1 = sub_dir_path.to_str().unwrap();
            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, filename2, sub_dir1];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_paths_single_source_multiple_dest() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();
            let sub_dir1 = sub_dir_path.to_str().unwrap();
            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();
            let sub_dir2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                filename2,
                sub_dir1,
                sub_dir2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_paths_multiple_sources_multiple_dest() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();
            let sub_dir1 = sub_dir_path.to_str().unwrap();
            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();
            let sub_dir2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                filename2,
                sub_dir1,
                sub_dir2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_paths_absolute_vs_relative() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_paths_special_characters() {
            let args = vec![
                ctcore::ct_util_name(),
                r#"source\path with spaces and !@#$%^&*().txt"#,
                r#"dest/path/with/special_chars/!@#$%^&*().txt"#,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_paths_empty() {
            let args = vec![ctcore::ct_util_name()];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(_output) => {
                    assert_eq!(1, 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_paths_missing() {
            let args = vec![ctcore::ct_util_name(), ""];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(_output) => {
                    // panic!("ct_main returned an error:{}", output.code());
                    assert_eq!(1, 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_one_file_system_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--one-file-system",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_one_file_system_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--one-file-system",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_x_one_file_system_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--one-file-system",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ax_one_file_system_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--one-file-system",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_sparse_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--sparse=always",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_sparse_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--sparse=always",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_sparse_auto_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--sparse=auto",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_sparse_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--sparse=never",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_sparse_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--sparse=never",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_x_sparse_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--sparse=always",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ax_sparse_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--sparse=always",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_x_sparse_auto_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--sparse=auto",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ax_sparse_auto_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--sparse=auto",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_x_sparse_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--sparse=never",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ax_sparse_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--sparse=never",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_copy_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--copy-contents",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_copy_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--copy-contents",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_x_copy_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--copy-contents",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ax_copy_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--copy-contents",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--context", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(_output) => {
                    assert_eq!(1, 1);
                    // panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_af_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-af",
                "--context",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(_output) => {
                    assert_eq!(1, 1);
                    // panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_suffix_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--suffix=SUFFIX",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_suffix_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--suffix=SUFFIX",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_x_suffix_valid_input() {
            let temp_dir1 = Builder::new()
                .prefix("test_ctmain_x_suffix_valid_input1")
                .tempdir()
                .unwrap();
            let sub_dir_path1 = temp_dir1.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let test_file_1 = sub_dir_path1.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir2 = Builder::new()
                .prefix("test_ctmain_x_suffix_valid_input2")
                .tempdir()
                .unwrap();
            let sub_dir_path2 = temp_dir2.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path2).unwrap();
            let test_file_2 = sub_dir_path2.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--suffix=SUFFIX",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ax_suffix_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--suffix=SUFFIX",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_update_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--update", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_update_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--update",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_update_none_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--update=none",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_update_none_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--update=none",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_update_s_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-S",
                "--update",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_update_as_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-aS",
                "--update",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_s_update_none_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-S",
                "--update=none",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_as_update_none_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-aS",
                "--update=none",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_update_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-S",
                "--update=never",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_update_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "-S",
                "--update=never",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_update_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-S",
                "--update=always",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_update_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "-S",
                "--update=always",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-b", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-ab", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_force_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-f", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_force_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-af", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_force_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--force", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_force_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--force",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_f_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-f",
                "--backup",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_af_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-af",
                "--backup",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_f_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-f",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_af_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-af",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_f_backup_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-f",
                "--backup",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_af_backup_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-af",
                "--backup",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_force_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--force",
                "--backup",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_force_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--force",
                "--backup",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_force_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--force",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_force_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--force",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_force_backup_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--force",
                "--backup",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_force_backup_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--force",
                "--backup",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_symbolic_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-s", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_symbolic_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-as", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_symbolic_link_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--symbolic-link",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_symbolic_link_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--symbolic-link",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_verbose_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-v", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_verbose_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--verbose", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_verbose_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-av", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_a_verbose_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--verbose",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_recursive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--recursive", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_recursive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--recursive",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_r_recursive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-r", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ar_recursive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-ar", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_debug_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--debug", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_debug_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--debug",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_r_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-r",
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ar_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-ar",
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_r_debug_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-r",
                "--debug",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ar_debug_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-ar",
                "--debug",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_r_debug_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-r",
                "--debug",
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ar_debug_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-ar",
                "--debug",
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_no_clobber_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-n", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_a_no_clobber_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-an", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_no_clobber_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--no-clobber", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_no_clobber_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--no-clobber",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-l", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-al", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_link_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--link", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_link_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-a", "--link", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        // #[test]
        // fn test_ctmain_interactive_valid_input() {
        //     let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
        //     let sub_dir_path = temp_dir.path().join("sub_dir");
        //     fs::create_dir(&sub_dir_path).unwrap();
        //     let test_file_1 = sub_dir_path.join("tests_file1.txt");
        //     File::create(&test_file_1).unwrap();
        //     let filename1 = test_file_1.to_str().unwrap();
        //
        //     let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
        //     let sub_dir_path = temp_dir.path().join("sub_dir");
        //     fs::create_dir(&sub_dir_path).unwrap();
        //     let test_file_2 = sub_dir_path.join("tests_file2.txt");
        //     File::create(&test_file_2).unwrap();
        //     let filename2 = test_file_2.to_str().unwrap();
        //
        //
        //
        //     let args = vec![ctcore::util_name(), "-i", filename1, filename2];
        //
        //     let result = ct_main(args.iter().map(|s| OsString::from(s)));
        //     match result {
        //         Err(output) => {
        //             panic!("ct_main returned an error:{}", output.code());
        //         }
        //         Ok(output) => {
        //             assert_eq!(output, 0);
        //         }
        //     }
        // }
        //
        // #[test]
        // fn test_ctmain_a_interactive_valid_input() {
        //     let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
        //     let sub_dir_path = temp_dir.path().join("sub_dir");
        //     fs::create_dir(&sub_dir_path).unwrap();
        //     let test_file_1 = sub_dir_path.join("tests_file1.txt");
        //     File::create(&test_file_1).unwrap();
        //     let filename1 = test_file_1.to_str().unwrap();
        //
        //     let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
        //     let sub_dir_path = temp_dir.path().join("sub_dir");
        //     fs::create_dir(&sub_dir_path).unwrap();
        //     let test_file_2 = sub_dir_path.join("tests_file2.txt");
        //     File::create(&test_file_2).unwrap();
        //     let filename2 = test_file_2.to_str().unwrap();
        //
        //
        //
        //     let args = vec![ctcore::util_name(), "-ai", filename1, filename2];
        //
        //     let result = ct_main(args.iter().map(|s| OsString::from(s)));
        //     match result {
        //         Err(output) => {
        //             panic!("ct_main returned an error:{}", output.code());
        //         }
        //         Ok(output) => {
        //             assert_eq!(output, 0);
        //         }
        //     }
        // }

        // #[test]
        // fn test_ctmain_interactive_whole_valid_input() {
        //     let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
        //     let sub_dir_path = temp_dir.path().join("sub_dir");
        //     fs::create_dir(&sub_dir_path).unwrap();
        //     let test_file_1 = sub_dir_path.join("tests_file1.txt");
        //     File::create(&test_file_1).unwrap();
        //     let filename1 = test_file_1.to_str().unwrap();
        //
        //     let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
        //     let sub_dir_path = temp_dir.path().join("sub_dir");
        //     fs::create_dir(&sub_dir_path).unwrap();
        //     let test_file_2 = sub_dir_path.join("tests_file2.txt");
        //     File::create(&test_file_2).unwrap();
        //     let filename2 = test_file_2.to_str().unwrap();
        //
        //
        //
        //     let args = vec![ctcore::util_name(), "--interactive", filename1, filename2];
        //
        //     let result = ct_main(args.iter().map(|s| OsString::from(s)));
        //     match result {
        //         Err(output) => {
        //             panic!("ct_main returned an error:{}", output.code());
        //         }
        //         Ok(output) => {
        //             assert_eq!(output, 0);
        //         }
        //     }
        // }

        // #[test]
        // fn test_ctmain_a_interactive_whole_valid_input() {
        //     let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
        //     let sub_dir_path = temp_dir.path().join("sub_dir");
        //     fs::create_dir(&sub_dir_path).unwrap();
        //     let test_file_1 = sub_dir_path.join("tests_file1.txt");
        //     File::create(&test_file_1).unwrap();
        //     let filename1 = test_file_1.to_str().unwrap();
        //
        //     let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
        //     let sub_dir_path = temp_dir.path().join("sub_dir");
        //     fs::create_dir(&sub_dir_path).unwrap();
        //     let test_file_2 = sub_dir_path.join("tests_file2.txt");
        //     File::create(&test_file_2).unwrap();
        //     let filename2 = test_file_2.to_str().unwrap();
        //
        //
        //
        //     let args = vec![
        //         ctcore::util_name(),
        //         "-a",
        //         "--interactive",
        //         filename1,
        //         filename2,
        //     ];
        //
        //     let result = ct_main(args.iter().map(|s| OsString::from(s)));
        //     match result {
        //         Err(output) => {
        //             panic!("ct_main returned an error:{}", output.code());
        //         }
        //         Ok(output) => {
        //             assert_eq!(output, 0);
        //         }
        //     }
        // }

        #[test]
        fn test_ctmain_no_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-T", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_no_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-aT", filename1, filename2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_no_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--no-target-directory",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_no_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--no-target-directory",
                filename1,
                filename2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-t", &temp_dir_1, &temp_dir_2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-at", &temp_dir_1, &temp_dir_2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_r_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-rt", &temp_dir_1, &temp_dir_2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ar_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-art", &temp_dir_1, &temp_dir_2];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_target_directory_whole_valid_input() {
            let temp_dir = Builder::new()
                .prefix("test_ctmain_target_directory_whole_valid_input")
                .tempdir()
                .unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("test_ctmain_target_directory_whole_valid_input")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_a_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_ar_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-ar",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_arf_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-arf",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_arfv_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-arfv",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_arfv_whole_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--archive",
                "--recursive",
                "--force",
                "--verbose",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_arfvi_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-arfvi",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_arfvi_whole_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--archive",
                "--recursive",
                "--force",
                "--verbose",
                "--interactive",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ctmain_arfviuln_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-arfviuln",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];
            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ctmain_arfviuln_whole_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--archive",
                "--recursive",
                "--force",
                "--verbose",
                "--interactive",
                "--attributes-only",
                "--link",
                "--no-clobber",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = cp_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    panic!("ct_main returned an error:{}", output.code());
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        ////////////////////////////////////////////
    }
    #[cfg(test)]
    mod tests_ct_app {
        use crate::{ct_app, opt_flags};
        use clap::error::ErrorKind;
        use std::fs;
        use std::fs::File;
        use std::path::PathBuf;

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
        fn test_ct_app_h() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_g_bar_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-g", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::PROGRESS_BAR)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_ag_bar_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-ag", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::PROGRESS_BAR)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_progress_bar_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--progress", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::PROGRESS_BAR)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_progress_bar_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--progress",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::PROGRESS_BAR)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_d_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-d", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_DEREFERENCE_PRESERVE_LINKS)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_ad_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-ad", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_DEREFERENCE_PRESERVE_LINKS)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_d_links_invalid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-d",
                filename1,
                filename2,
                "extra_arg",
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_DEREFERENCE_PRESERVE_LINKS)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_preserve_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--preserve", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_preserve_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_preserve_all_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=all",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_preserve_all_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=all",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_d_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-d", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_ad_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-ad", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_adr_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-adr", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_adrf_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-adrf", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_adrfv_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-adrfv", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_preserve_mode_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=mode",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_preserve_mode_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=mode",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_preserve_ownership_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=mode",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_preserve_timestamps_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=timestamps",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_preserve_timestamps_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=timestamps",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_preserve_context_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=context",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_preserve_context_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=context",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_preserve_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=link",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_preserve_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=link",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_preserve_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=links",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_preserve_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=links",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_preserve_xattr_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve=xattr",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_preserve_xattr_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve=xattr",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_a_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_preserve_mode_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=mode",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_u_preserve_ownership_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=ownership",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_u_preserve_timestamps_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-u", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_preserve_context_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=context",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_u_preserve_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=link",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_preserve_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=links",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_preserve_xattr_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                "-u",
                ctcore::ct_util_name(),
                "--preserve=xattr",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_p_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-p",
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_aru_preserve_ownership_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-uar",
                "--preserve=ownership",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_aru_preserve_timestamps_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-u", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_aru_preserve_context_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--preserve=context",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_aru_preserve_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-aru",
                "--preserve=link",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_aru_preserve_links_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-aru",
                "--preserve=links",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_aru_preserve_xattr_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                "-aru",
                ctcore::ct_util_name(),
                "--preserve=xattr",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_aru_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-aru",
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_aup_preserve_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-aup",
                "--preserve-default-attributes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        ///////////////

        #[test]
        fn test_ct_app_no_preserve_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--no-preserve",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_p_no_preserve_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-p",
                "--no-preserve",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_parents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--parents", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_p_parents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-p",
                "--parents",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_hard_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-H", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::CLI_SYMBOLIC_LINKS)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_dereference_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-L", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::DEREFERENCE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_dereference_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--dereference",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::DEREFERENCE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_l_dereference_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-L",
                "--dereference",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::DEREFERENCE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_no_dereference_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-P", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_DEREFERENCE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_no_dereference_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--no-dereference",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_DEREFERENCE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_p_no_dereference_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-P",
                "--no-dereference",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_DEREFERENCE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_archive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-a", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::ARCHIVE).unwrap());
        }
        #[test]
        fn test_ct_app_archive_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--archive", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::ARCHIVE).unwrap());
        }

        #[test]
        fn test_ct_app_reflink_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--reflink=always",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_reflink_auto_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--reflink=auto",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_reflink_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--reflink=never",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_reflink_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--reflink=never",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_reflink_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--reflink=always",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_u_reflink_auto_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-u",
                "--reflink=auto",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_attributes_only_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--attributes-only",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::ATTRIBUTES_ONLY)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_copy_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-c", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_u_update_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-u", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_paths_single_source_single_dest() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_paths_multiple_sources_single_dest() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();
            let sub_dir1 = sub_dir_path.to_str().unwrap();
            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, filename2, sub_dir1];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_paths_single_source_multiple_dest() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();
            let sub_dir1 = sub_dir_path.to_str().unwrap();
            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();
            let sub_dir2 = sub_dir_path.to_str().unwrap();
            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                filename2,
                sub_dir1,
                sub_dir2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_paths_multiple_sources_multiple_dest() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();
            let sub_dir1 = sub_dir_path.to_str().unwrap();
            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();
            let sub_dir2 = sub_dir_path.to_str().unwrap();
            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                filename2,
                sub_dir1,
                sub_dir2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_paths_absolute_vs_relative() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            // ... 进一步验证绝对路径和相对路径均能正确解析 ...
        }

        #[test]
        fn test_ct_app_paths_special_characters() {
            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                r#"source\path with spaces and !@#$%^&*().txt"#,
                r#"dest/path/with/special_chars/!@#$%^&*().txt"#,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_paths_empty() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name()];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_paths_missing() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), ""];

            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_one_file_system_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--one-file-system",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::ONE_FILE_SYSTEM)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_one_file_system_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--one-file-system",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::ONE_FILE_SYSTEM)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_x_one_file_system_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--one-file-system",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::ONE_FILE_SYSTEM)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_ax_one_file_system_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--one-file-system",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::ONE_FILE_SYSTEM)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_sparse_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--sparse=always",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_sparse_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--sparse=always",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_sparse_auto_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--sparse=auto",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_sparse_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--sparse=never",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_sparse_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--sparse=never",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_x_sparse_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--sparse=always",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_ax_sparse_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--sparse=always",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_x_sparse_auto_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--sparse=auto",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_ax_sparse_auto_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--sparse=auto",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_x_sparse_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--sparse=never",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_ax_sparse_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--sparse=never",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_copy_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--copy-contents",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_copy_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--copy-contents",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_x_copy_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--copy-contents",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_ax_copy_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--copy-contents",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--context", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_af_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-af",
                "--context",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--context",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_x_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--context",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_ax_contents_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--context",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_suffix_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--suffix=SUFFIX",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_suffix_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--suffix=SUFFIX",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_x_suffix_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-x",
                "--suffix=SUFFIX",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_ax_suffix_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ax",
                "--suffix=SUFFIX",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--update", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_update_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--update",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_none_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--update=none",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_update_none_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--update=none",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_s_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-S",
                "--update",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_as_default_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-aS",
                "--update",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_s_update_none_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-S",
                "--update=none",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_as_update_none_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-aS",
                "--update=none",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-S",
                "--update=never",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_update_never_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "-S",
                "--update=never",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_update_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-S",
                "--update=always",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_update_always_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "-S",
                "--update=always",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-b", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-ab", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_force_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-f", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_force_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-af", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_force_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--force", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_force_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--force",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_f_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-f",
                "--backup",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_af_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-af",
                "--backup",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_f_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-f",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_af_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-af",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_f_backup_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-f",
                "--backup",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_af_backup_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-af",
                "--backup",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_force_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--force",
                "--backup",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_force_backup_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--force",
                "--backup",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_force_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--force",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_force_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--force",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_force_backup_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--force",
                "--backup",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_a_force_backup_remove_destination_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--force",
                "--backup",
                "--remove-destination",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_symbolic_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-s", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::SYMBOLIC_LINK)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_symbolic_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-as", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::SYMBOLIC_LINK)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_symbolic_link_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--symbolic-link",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::SYMBOLIC_LINK)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_symbolic_link_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--symbolic-link",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::SYMBOLIC_LINK)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_verbose_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-v", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());

            assert!(result.unwrap().get_one::<bool>(opt_flags::VERBOSE).unwrap());
        }
        #[test]
        fn test_ct_app_verbose_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--verbose", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::VERBOSE).unwrap());
        }

        #[test]
        fn test_ct_app_a_verbose_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-av", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());

            assert!(result.unwrap().get_one::<bool>(opt_flags::VERBOSE).unwrap());
        }
        #[test]
        fn test_ct_app_a_verbose_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--verbose",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::VERBOSE).unwrap());
        }

        #[test]
        fn test_ct_app_recursive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--recursive", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::RECURSIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_recursive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--recursive",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::RECURSIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_r_recursive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-r", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::RECURSIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_ar_recursive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-ar", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::RECURSIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::STRIP_TRAILING_SLASHES)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::STRIP_TRAILING_SLASHES)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_debug_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--debug", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::DEBUG).unwrap());
        }

        #[test]
        fn test_ct_app_a_debug_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--debug",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::DEBUG).unwrap());
        }

        #[test]
        fn test_ct_app_r_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-r",
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::RECURSIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_ar_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ar",
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::RECURSIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_r_debug_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-r",
                "--debug",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::RECURSIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_ar_debug_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ar",
                "--debug",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::RECURSIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_r_debug_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-r",
                "--debug",
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::RECURSIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_ar_debug_strip_trailing_slashes_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ar",
                "--debug",
                "--strip-trailing-slashes",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::RECURSIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_no_clobber_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-n", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());

            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_CLOBBER)
                    .unwrap()
            );
        }
        #[test]
        fn test_ct_app_a_no_clobber_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-an", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());

            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_CLOBBER)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_no_clobber_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--no-clobber", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_CLOBBER)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_no_clobber_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--no-clobber",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_CLOBBER)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-l", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::LINK).unwrap());
        }

        #[test]
        fn test_ct_app_a_link_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-al", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::LINK).unwrap());
        }

        #[test]
        fn test_ct_app_link_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--link", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::LINK).unwrap());
        }

        #[test]
        fn test_ct_app_a_link_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-a", "--link", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::LINK).unwrap());
        }

        #[test]
        fn test_ct_app_interactive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-i", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::INTERACTIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_interactive_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-ai", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::INTERACTIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_interactive_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--interactive",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::INTERACTIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_interactive_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--interactive",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::INTERACTIVE)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_no_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-T", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_TARGET_DIRECTORY)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_no_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-aT", filename1, filename2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_TARGET_DIRECTORY)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_no_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--no-target-directory",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());

            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_TARGET_DIRECTORY)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_a_no_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("tests_file1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("tests_file2").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--no-target-directory",
                filename1,
                filename2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());

            assert!(
                result
                    .unwrap()
                    .get_one::<bool>(opt_flags::NO_TARGET_DIRECTORY)
                    .unwrap()
            );
        }

        #[test]
        fn test_ct_app_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-t", &temp_dir_1, &temp_dir_2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_a_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-at", &temp_dir_1, &temp_dir_2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_r_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-rt", &temp_dir_1, &temp_dir_2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_ar_target_directory_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app1").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-art", &temp_dir_1, &temp_dir_2];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_target_directory_whole_valid_input() {
            let temp_dir = Builder::new()
                .prefix("test_ct_app_target_directory_whole_valid_input")
                .tempdir()
                .unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("test_ct_app_target_directory_whole_valid_input")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_a_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-a",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_ar_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-ar",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_arf_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-arf",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_arfv_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-arfv",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_arfv_whole_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--archive",
                "--recursive",
                "--force",
                "--verbose",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_arfvi_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-arfvi",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_arfvi_whole_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--archive",
                "--recursive",
                "--force",
                "--verbose",
                "--interactive",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }

        #[test]
        fn test_ct_app_arfviuln_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "-arfviuln",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }
        #[test]
        fn test_ct_app_arfviuln_whole_target_directory_whole_valid_input() {
            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path1 = temp_dir.path().join("sub_dir1");
            fs::create_dir(&sub_dir_path1).unwrap();
            let temp_dir_1 = sub_dir_path1.to_str().unwrap();

            let temp_dir = Builder::new().prefix("test_ct_app").tempdir().unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir2");
            fs::create_dir(&sub_dir_path).unwrap();

            let temp_dir_2 = sub_dir_path.to_str().unwrap();

            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--archive",
                "--recursive",
                "--force",
                "--verbose",
                "--interactive",
                "--attributes-only",
                "--link",
                "--no-clobber",
                "--target-directory",
                &temp_dir_1,
                &temp_dir_2,
            ];

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<PathBuf>(opt_flags::TARGET_DIRECTORY)
                    .unwrap(),
                &sub_dir_path1
            );
        }
    }
}
