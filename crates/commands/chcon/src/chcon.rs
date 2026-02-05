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
#![allow(clippy::upper_case_acronyms)]

extern crate rust_i18n;
use clap::builder::ValueParser;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError};
use ctcore::{ct_display::Quotable, ct_show_error, ct_show_warning};

use clap::{Arg, ArgAction, Command, crate_version};
use selinux::{OpaqueSecurityContext, SecurityContext};

use rust_i18n::t;
use std::borrow::Cow;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::{fs, io};
use sys_locale::get_locale;
rust_i18n::i18n!("locales", fallback = "en-US");

mod errors;
mod fts;

use ctcore::Tool;
use errors::*;

pub mod opt_flags {
    pub static HELP: &str = "help";
    pub static VERBOSE: &str = "verbose";

    pub static REFERENCE: &str = "reference";

    pub static USER: &str = "user";
    pub static ROLE: &str = "role";
    pub static TYPE: &str = "type";
    pub static RANGE: &str = "range";

    pub static RECURSIVE: &str = "recursive";

    pub mod sym_links {
        pub static FOLLOW_ARG_DIR_SYM_LINK: &str = "follow-arg-dir-sym-link";
        pub static FOLLOW_DIR_SYM_LINKS: &str = "follow-dir-sym-links";
        pub static NO_FOLLOW_SYM_LINKS: &str = "no-follow-sym-links";
    }

    pub mod dereference {
        pub static DEREFERENCE: &str = "dereference";
        pub static NO_DEREFERENCE: &str = "no-dereference";
    }

    pub mod preserve_root {
        pub static PRESERVE_ROOT: &str = "preserve-root";
        pub static NO_PRESERVE_ROOT: &str = "no-preserve-root";
    }
}

pub fn chcon_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let config_info = ct_app();

    let opt_flags = match chcon_parse_command_line(config_info, args) {
        Ok(r) => r,
        Err(r) => {
            if let Error::CommandLine(r) = r {
                return Err(r.into());
            }

            return Err(CTsageError::new(libc::EXIT_FAILURE, format!("{r}.\n")));
        }
    };

    let security_context = match &opt_flags.mode {
        ChconCommandLineMode::ReferenceBased { reference } => {
            let result = match SecurityContext::of_path(reference, true, false) {
                Ok(Some(context)) => Ok(context),

                Ok(None) => {
                    let err = io::Error::from_raw_os_error(libc::ENODATA);
                    Err(Error::from_io1("Getting security context", reference, err))
                }

                Err(r) => Err(Error::from_selinux("Getting security context", r)),
            };

            match result {
                Err(r) => {
                    return Err(CtSimpleError::new(
                        libc::EXIT_FAILURE,
                        format!("{}.", report_full_error(&r)),
                    ));
                }

                Ok(file_context) => ChconSELinuxSecurityContext::File(file_context),
            }
        }

        ChconCommandLineMode::ContextBased { context } => {
            let c_context = match chcon_os_str_to_c_string(context) {
                Ok(context) => context,

                Err(_r) => {
                    return Err(CtSimpleError::new(
                        libc::EXIT_FAILURE,
                        format!("Invalid security context {}.", context.quote()),
                    ));
                }
            };

            if SecurityContext::from_c_str(&c_context, false).check() == Some(false) {
                return Err(CtSimpleError::new(
                    libc::EXIT_FAILURE,
                    format!("Invalid security context {}.", context.quote()),
                ));
            }

            ChconSELinuxSecurityContext::String(Some(c_context))
        }

        ChconCommandLineMode::Custom { .. } => ChconSELinuxSecurityContext::String(None),
    };

    let root_device_info = if opt_flags.preserve_root && opt_flags.recursive_mode.is_recursive() {
        match chcon_get_root_dev_ino() {
            Ok(r) => Some(r),

            Err(r) => {
                return Err(CtSimpleError::new(
                    libc::EXIT_FAILURE,
                    format!("{}.", report_full_error(&r)),
                ));
            }
        }
    } else {
        None
    };

    let results = chcon_process_files(&opt_flags, &security_context, root_device_info);
    if results.is_empty() {
        return Ok(());
    }

    for result in &results {
        ct_show_error!("{}.", report_full_error(result));
    }
    Err(libc::EXIT_FAILURE.into())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("chcon.about");
    let usage_description = t!("chcon.usage");

    let args = chcon_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args_override_self(true)
        .args(&args)
}

fn chcon_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(opt_flags::HELP)
            .long(opt_flags::HELP)
            .help(t!("chcon.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .short('V')
            .long("version")
            .help(t!("chcon.clap.version"))
            .action(ArgAction::Version),
        Arg::new(opt_flags::dereference::DEREFERENCE)
            .long(opt_flags::dereference::DEREFERENCE)
            .overrides_with(opt_flags::dereference::NO_DEREFERENCE)
            .help(t!("chcon.clap.dereference"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::dereference::NO_DEREFERENCE)
            .short('h')
            .long(opt_flags::dereference::NO_DEREFERENCE)
            .help(t!("chcon.clap.no_dereference"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::preserve_root::PRESERVE_ROOT)
            .long(opt_flags::preserve_root::PRESERVE_ROOT)
            .overrides_with(opt_flags::preserve_root::NO_PRESERVE_ROOT)
            .help(t!("chcon.clap.preserve_root"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::preserve_root::NO_PRESERVE_ROOT)
            .long(opt_flags::preserve_root::NO_PRESERVE_ROOT)
            .help(t!("chcon.clap.no_preserve_root"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::REFERENCE)
            .long(opt_flags::REFERENCE)
            .value_name("RFILE")
            .value_hint(clap::ValueHint::FilePath)
            .conflicts_with_all([
                opt_flags::USER,
                opt_flags::ROLE,
                opt_flags::TYPE,
                opt_flags::RANGE,
            ])
            .help(t!("chcon.clap.reference"))
            .value_parser(ValueParser::os_string()),
        Arg::new(opt_flags::USER)
            .short('u')
            .long(opt_flags::USER)
            .value_name("USER")
            .value_hint(clap::ValueHint::Username)
            .help(t!("chcon.clap.user"))
            .value_parser(ValueParser::os_string()),
        Arg::new(opt_flags::ROLE)
            .short('r')
            .long(opt_flags::ROLE)
            .value_name("ROLE")
            .help(t!("chcon.clap.role"))
            .value_parser(ValueParser::os_string()),
        Arg::new(opt_flags::TYPE)
            .short('t')
            .long(opt_flags::TYPE)
            .value_name("TYPE")
            .help(t!("chcon.clap.type"))
            .value_parser(ValueParser::os_string()),
        Arg::new(opt_flags::RANGE)
            .short('l')
            .long(opt_flags::RANGE)
            .value_name("RANGE")
            .help(t!("chcon.clap.range"))
            .value_parser(ValueParser::os_string()),
        Arg::new(opt_flags::RECURSIVE)
            .short('R')
            .long(opt_flags::RECURSIVE)
            .help(t!("chcon.clap.recursive"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::sym_links::FOLLOW_ARG_DIR_SYM_LINK)
            .short('H')
            .requires(opt_flags::RECURSIVE)
            .overrides_with_all([
                opt_flags::sym_links::FOLLOW_DIR_SYM_LINKS,
                opt_flags::sym_links::NO_FOLLOW_SYM_LINKS,
            ])
            .help(t!("chcon.clap.follow_arg_dir_sym_link"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::sym_links::FOLLOW_DIR_SYM_LINKS)
            .short('L')
            .requires(opt_flags::RECURSIVE)
            .overrides_with_all([
                opt_flags::sym_links::FOLLOW_ARG_DIR_SYM_LINK,
                opt_flags::sym_links::NO_FOLLOW_SYM_LINKS,
            ])
            .help(t!("chcon.clap.follow_dir_sym_links"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::sym_links::NO_FOLLOW_SYM_LINKS)
            .short('P')
            .requires(opt_flags::RECURSIVE)
            .overrides_with_all([
                opt_flags::sym_links::FOLLOW_ARG_DIR_SYM_LINK,
                opt_flags::sym_links::FOLLOW_DIR_SYM_LINKS,
            ])
            .help(t!("chcon.clap.no_follow_sym_links"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::VERBOSE)
            .short('v')
            .long(opt_flags::VERBOSE)
            .help(t!("chcon.clap.verbose"))
            .action(ArgAction::SetTrue),
        Arg::new("FILE")
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath)
            .num_args(1..)
            .value_parser(ValueParser::os_string()),
    ];
    args
}

#[derive(Debug)]
struct ChconOptions {
    verbose: bool,
    preserve_root: bool,
    recursive_mode: ChconRecursiveMode,
    affect_symlink_referent: bool,
    mode: ChconCommandLineMode,
    files: Vec<PathBuf>,
}

fn chcon_parse_command_line(
    cmd_config: clap::Command,
    args: impl ctcore::Args,
) -> Result<ChconOptions> {
    let args_match = cmd_config.try_get_matches_from(args)?;

    let is_verbose = args_match.get_flag(opt_flags::VERBOSE);

    let (recursive_mode, affect_symlink_referent) = if args_match.get_flag(opt_flags::RECURSIVE) {
        if args_match.get_flag(opt_flags::sym_links::FOLLOW_DIR_SYM_LINKS) {
            if args_match.get_flag(opt_flags::dereference::NO_DEREFERENCE) {
                return Err(Error::ArgumentsMismatch(format!(
                    "'--{}' with '--{}' require '-P'",
                    opt_flags::RECURSIVE,
                    opt_flags::dereference::NO_DEREFERENCE
                )));
            }

            (ChconRecursiveMode::RecursiveAndFollowAllDirSymLinks, true)
        } else if args_match.get_flag(opt_flags::sym_links::FOLLOW_ARG_DIR_SYM_LINK) {
            if args_match.get_flag(opt_flags::dereference::NO_DEREFERENCE) {
                return Err(Error::ArgumentsMismatch(format!(
                    "'--{}' with '--{}' require '-P'",
                    opt_flags::RECURSIVE,
                    opt_flags::dereference::NO_DEREFERENCE
                )));
            }

            (ChconRecursiveMode::RecursiveAndFollowArgDirSymLinks, true)
        } else {
            if args_match.get_flag(opt_flags::dereference::DEREFERENCE) {
                return Err(Error::ArgumentsMismatch(format!(
                    "'--{}' with '--{}' require either '-H' or '-L'",
                    opt_flags::RECURSIVE,
                    opt_flags::dereference::DEREFERENCE
                )));
            }

            (ChconRecursiveMode::RecursiveButDoNotFollowSymLinks, false)
        }
    } else {
        let no_dereference = args_match.get_flag(opt_flags::dereference::NO_DEREFERENCE);
        (ChconRecursiveMode::NotRecursive, !no_dereference)
    };

    // 默认情况下，不保留根目录。

    let match_preserve_root = args_match.get_flag(opt_flags::preserve_root::PRESERVE_ROOT);

    let mut match_files = args_match.get_many::<OsString>("FILE").unwrap_or_default();

    let command_mode = if let Some(path) = args_match.get_one::<OsString>(opt_flags::REFERENCE) {
        ChconCommandLineMode::ReferenceBased {
            reference: PathBuf::from(path),
        }
    } else if args_match.contains_id(opt_flags::USER)
        || args_match.contains_id(opt_flags::ROLE)
        || args_match.contains_id(opt_flags::TYPE)
        || args_match.contains_id(opt_flags::RANGE)
    {
        ChconCommandLineMode::Custom {
            user: args_match
                .get_one::<OsString>(opt_flags::USER)
                .map(Into::into),
            role: args_match
                .get_one::<OsString>(opt_flags::ROLE)
                .map(Into::into),
            the_type: args_match
                .get_one::<OsString>(opt_flags::TYPE)
                .map(Into::into),
            range: args_match
                .get_one::<OsString>(opt_flags::RANGE)
                .map(Into::into),
        }
    } else if let Some(context) = match_files.next() {
        ChconCommandLineMode::ContextBased {
            context: context.into(),
        }
    } else {
        return Err(Error::MissingContext);
    };

    let files: Vec<_> = match_files.map(PathBuf::from).collect();
    if files.is_empty() {
        return Err(Error::MissingFiles);
    }

    Ok(ChconOptions {
        verbose: is_verbose,
        preserve_root: match_preserve_root,
        recursive_mode,
        affect_symlink_referent,
        mode: command_mode,
        files,
    })
}

#[derive(Debug, Copy, Clone)]
enum ChconRecursiveMode {
    NotRecursive,
    /// Do not traverse any symbolic links.
    RecursiveButDoNotFollowSymLinks,
    /// Traverse every symbolic link to a directory encountered.
    RecursiveAndFollowAllDirSymLinks,
    /// If a command line argument is a symbolic link to a directory, traverse it.
    RecursiveAndFollowArgDirSymLinks,
}

impl ChconRecursiveMode {
    fn is_recursive(self) -> bool {
        match self {
            Self::NotRecursive => false,

            Self::RecursiveButDoNotFollowSymLinks
            | Self::RecursiveAndFollowAllDirSymLinks
            | Self::RecursiveAndFollowArgDirSymLinks => true,
        }
    }

    fn fts_open_options(self) -> c_int {
        match self {
            Self::NotRecursive | Self::RecursiveButDoNotFollowSymLinks => fts_sys::FTS_PHYSICAL,

            Self::RecursiveAndFollowAllDirSymLinks => fts_sys::FTS_LOGICAL,

            Self::RecursiveAndFollowArgDirSymLinks => {
                fts_sys::FTS_PHYSICAL | fts_sys::FTS_COMFOLLOW
            }
        }
    }
}

#[derive(Debug)]
enum ChconCommandLineMode {
    ReferenceBased {
        reference: PathBuf,
    },
    ContextBased {
        context: OsString,
    },
    Custom {
        user: Option<OsString>,
        role: Option<OsString>,
        the_type: Option<OsString>,
        range: Option<OsString>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChconDeviceAndINode {
    device_id: u64,
    inode: u64,
}

#[cfg(unix)]
impl From<fs::Metadata> for ChconDeviceAndINode {
    fn from(md: fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;

        Self {
            device_id: md.dev(),
            inode: md.ino(),
        }
    }
}

impl TryFrom<&libc::stat> for ChconDeviceAndINode {
    type Error = Error;

    #[allow(clippy::useless_conversion)]
    fn try_from(st: &libc::stat) -> Result<Self> {
        let device_id = u64::try_from(st.st_dev).map_err(|_r| Error::OutOfRange)?;
        let inode = u64::try_from(st.st_ino).map_err(|_r| Error::OutOfRange)?;
        Ok(Self { device_id, inode })
    }
}

/// 对指定文件或目录应用SELinux上下文。
///
/// # 返回值
/// 返回一个错误向量，其中包含操作过程中发生的任何错误。
fn chcon_process_files(
    chcon_options: &ChconOptions,
    security_context: &ChconSELinuxSecurityContext,
    chcon_device_inode: Option<ChconDeviceAndINode>,
) -> Vec<Error> {
    // 使用fts选项打开文件树，用于递归处理文件。
    let fts_options = chcon_options.recursive_mode.fts_open_options();
    let mut fts_info = match fts::FTS::new(chcon_options.files.iter(), fts_options) {
        Ok(fts) => fts,
        Err(err) => return vec![err], // 如果无法初始化FTS，直接返回错误。
    };

    let mut chcon_errors = Vec::default();
    loop {
        match fts_info.read_next_entry() {
            Ok(true) => {
                // 处理当前文件或目录的SELinux上下文。
                if let Err(err) = chcon_process_file(
                    chcon_options,
                    security_context,
                    &mut fts_info,
                    chcon_device_inode,
                ) {
                    chcon_errors.push(err); // 收集处理过程中的任何错误。
                }
            }

            Ok(false) => break, // 如果没有更多条目，则结束循环。

            Err(err) => {
                chcon_errors.push(err); // 收集读取条目时发生的任何错误。
                break;
            }
        }
    }
    chcon_errors // 返回收集到的错误。
}
/**
 * 更改文件的安全上下文。
 *
 * 此函数使用 `fts` 系统遍历文件树，并对指定的文件或目录应用新的安全上下文。
 * 它处理各种文件类型和情况，包括递归更改、错误处理以及避免更改根目录的权限。
 *
 * @param chcon_options 包含更改上下文选项的结构体。
 * @param security_context 指定要应用的新安全上下文。
 * @param fts_info 文件树遍历状态的结构体，用于控制和获取遍历信息。
 * @param chcon_device_inode 设备号和 inode 号的选项，用于检查是否尝试更改根目录的上下文。
 * @return Result<()>，如果成功则返回 Ok(())，如果遇到错误则返回 Err(())。
 */
fn chcon_process_file(
    chcon_options: &ChconOptions,
    security_context: &ChconSELinuxSecurityContext,
    fts_info: &mut fts::FTS,
    chcon_device_inode: Option<ChconDeviceAndINode>,
) -> Result<()> {
    // 获取当前遍历的文件引用。
    let mut entry_ref = fts_info.last_entry_ref().unwrap();

    // 验证文件名并获取文件的访问路径。
    let file_path_name = entry_ref.path().map(PathBuf::from).ok_or_else(|| {
        Error::from_io("File name validation", io::ErrorKind::InvalidInput.into())
    })?;

    let fts_path_info = entry_ref.access_path().ok_or_else(|| {
        let err = io::ErrorKind::InvalidInput.into();
        Error::from_io1("File name validation", &file_path_name, err)
    })?;

    // 准备错误处理。
    let err = |s, k: io::ErrorKind| Error::from_io1(s, &file_path_name, k.into());
    let fts_err = |s| {
        let r = io::Error::from_raw_os_error(entry_ref.errno());
        Err(Error::from_io1(s, &file_path_name, r))
    };

    // 获取文件的设备号和 inode 号，用于后续的检查。
    let file_dev_ino: ChconDeviceAndINode = if let Some(st) = entry_ref.stat() {
        st.try_into()?
    } else {
        return Err(err("Getting meta data", io::ErrorKind::InvalidInput));
    };

    let mut result = Ok(());

    // 根据文件类型和遍历选项执行相应的处理逻辑。
    match entry_ref.flags() {
        fts_sys::FTS_D => {
            // 处理目录，决定是否递归更改。
            if chcon_options.recursive_mode.is_recursive() {
                if chcon_root_dev_ino_check(chcon_device_inode, file_dev_ino) {
                    chcon_root_dev_ino_warn(&file_path_name);
                    let _ = fts_info.set(fts_sys::FTS_SKIP);
                    let _ = fts_info.read_next_entry();
                    return Err(err("Modifying root path", io::ErrorKind::PermissionDenied));
                }
                return Ok(());
            }
        }

        fts_sys::FTS_DP => {
            // 已访问过的目录，根据是否递归进行处理。
            if !chcon_options.recursive_mode.is_recursive() {
                return Ok(());
            }
        }

        fts_sys::FTS_NS => {
            // 处理 stat 失败的情况，尝试重新进行 stat。
            if entry_ref.level() == 0 && entry_ref.number() == 0 {
                entry_ref.set_number(1);
                let _ignored = fts_info.set(fts_sys::FTS_AGAIN);
                return Ok(());
            }

            result = fts_err("Accessing");
        }

        fts_sys::FTS_ERR => result = fts_err("Accessing"),

        fts_sys::FTS_DNR => result = fts_err("Reading directory"),

        fts_sys::FTS_DC => {
            // 检测并处理循环引用的目录。
            if chcon_cycle_warning_required(
                chcon_options.recursive_mode.fts_open_options(),
                &entry_ref,
            ) {
                chcon_emit_cycle_warning(&file_path_name);
                return Err(err("Reading cyclic directory", io::ErrorKind::InvalidData));
            }
        }

        _ => {}
    }

    // 在处理完文件类型和遍历逻辑后，检查是否需要更改文件的安全上下文。
    if entry_ref.flags() == fts_sys::FTS_DP
        && result.is_ok()
        && chcon_root_dev_ino_check(chcon_device_inode, file_dev_ino)
    {
        chcon_root_dev_ino_warn(&file_path_name);
        result = Err(err("Modifying root path", io::ErrorKind::PermissionDenied));
    }

    // 如果之前没有错误，则尝试更改文件的安全上下文。
    if result.is_ok() {
        if chcon_options.verbose {
            println!(
                "{}: Changing security context of: {}",
                ctcore::ct_util_name(),
                file_path_name.quote()
            );
        }

        result = chcon_change_file_context(chcon_options, security_context, fts_path_info);
    }

    // 如果是非递归模式，设置 FTS_SKIP 以跳过剩余的文件。
    if !chcon_options.recursive_mode.is_recursive() {
        let _ignored = fts_info.set(fts_sys::FTS_SKIP);
    }
    result
}
/**
 * 更改文件的安全上下文。
 *
 * 此函数根据提供的 `ChconOptions` 和 `SELinuxSecurityContext` 对指定路径的文件（或符号链接）应用新的安全上下文。
 * 可以通过直接指定上下文或以引用/上下文为基础的方式来进行更改。
 *
 * @param chcon_options 提供有关如何更改安全上下文的选项，例如是否影响符号链接的目标。
 * @param security_context 指定要应用于文件的新安全上下文。
 * @param chcon_path 指定要更改其安全上下文的文件路径。
 * @return `Result<()>`，成功时返回 `Ok(())`，失败时返回包含错误信息的 `Err`。
 */
fn chcon_change_file_context(
    chcon_options: &ChconOptions,
    security_context: &ChconSELinuxSecurityContext,
    chcon_path: &Path,
) -> Result<()> {
    // 根据 chcon_options.mode 的值来决定如何处理安全上下文的更改。
    match &chcon_options.mode {
        ChconCommandLineMode::Custom {
            user,
            role,
            the_type,
            range,
        } => {
            // 如果文件没有上下文，并且我们没有设置所有上下文组件，则没有明显的默认值，因此直接放弃。
            let err0 = || -> Result<()> {
                let op = "Applying partial security context to unlabeled file";
                let err = io::ErrorKind::InvalidInput.into();
                Err(Error::from_io1(op, chcon_path, err))
            };

            // 尝试获取文件当前的安全上下文。
            let file_context = match SecurityContext::of_path(
                chcon_path,
                chcon_options.affect_symlink_referent,
                false,
            ) {
                Ok(Some(context)) => context,
                Ok(None) => return err0(),
                Err(r) => return Err(Error::from_selinux("Getting security context", r)),
            };

            // 将文件上下文转换为C字符串，用于后续处理。
            let c_file_context = match file_context.to_c_string() {
                Ok(Some(context)) => context,
                Ok(None) => return err0(),
                Err(r) => return Err(Error::from_selinux("Getting security context", r)),
            };

            // 创建一个不透明的安全上下文对象，用于设置新的上下文值。
            let se_context =
                OpaqueSecurityContext::from_c_str(c_file_context.as_ref()).map_err(|_r| {
                    let err = io::ErrorKind::InvalidInput.into();
                    Error::from_io1("Creating security context", chcon_path, err)
                })?;

            // 定义一个类型为函数指针的枚举，用于设置安全上下文的不同组件。
            type SetValueProc = fn(&OpaqueSecurityContext, &CStr) -> selinux::errors::Result<()>;

            // 准备用于设置上下文组件的值和相应的函数指针。
            let list: &[(&Option<OsString>, SetValueProc)] = &[
                (user, OpaqueSecurityContext::set_user),
                (role, OpaqueSecurityContext::set_role),
                (the_type, OpaqueSecurityContext::set_type),
                (range, OpaqueSecurityContext::set_range),
            ];

            // 遍历列表，为安全上下文设置新的值。
            for (new_value, set_value_proc) in list {
                if let Some(new_value) = new_value {
                    let c_new_value = chcon_os_str_to_c_string(new_value).map_err(|_r| {
                        let err = io::ErrorKind::InvalidInput.into();
                        Error::from_io1("Creating security context", chcon_path, err)
                    })?;

                    set_value_proc(&se_context, &c_new_value)
                        .map_err(|r| Error::from_selinux("Setting security context user", r))?;
                }
            }

            // 将最终的安全上下文转换为C字符串，准备应用到文件。
            let context_string = se_context
                .to_c_string()
                .map_err(|r| Error::from_selinux("Getting security context", r))?;

            // 检查文件的当前上下文是否已经是我们要设置的上下文，如果是则无需更改。
            if c_file_context.as_ref().to_bytes() == context_string.as_ref().to_bytes() {
                Ok(()) // 无需更改。
            } else {
                // 应用新的安全上下文到文件。
                SecurityContext::from_c_str(&context_string, false)
                    .set_for_path(chcon_path, chcon_options.affect_symlink_referent, false)
                    .map_err(|r| Error::from_selinux("Setting security context", r))
            }
        }

        ChconCommandLineMode::ReferenceBased { .. } | ChconCommandLineMode::ContextBased { .. } => {
            // 在这两种模式下，直接使用 `security_context` 设置文件的安全上下文。
            if let Some(c_context) = security_context.to_c_string()? {
                SecurityContext::from_c_str(c_context.as_ref(), false)
                    .set_for_path(chcon_path, chcon_options.affect_symlink_referent, false)
                    .map_err(|r| Error::from_selinux("Setting security context", r))
            } else {
                // 如果无法将安全上下文转换为C字符串，则报错。
                let err = io::ErrorKind::InvalidInput.into();
                Err(Error::from_io1("Setting security context", chcon_path, err))
            }
        }
    }
}

#[cfg(unix)]
pub(crate) fn chcon_os_str_to_c_string(os_str: &OsStr) -> Result<CString> {
    use std::os::unix::ffi::OsStrExt;

    CString::new(os_str.as_bytes())
        .map_err(|_r| Error::from_io("CString::new()", io::ErrorKind::InvalidInput.into()))
}

/// Call `lstat()` to get the device and inode numbers for `/`.
#[cfg(unix)]
fn chcon_get_root_dev_ino() -> Result<ChconDeviceAndINode> {
    fs::symlink_metadata("/")
        .map(ChconDeviceAndINode::from)
        .map_err(|r| Error::from_io1("std::fs::symlink_metadata", "/", r))
}

fn chcon_root_dev_ino_check(
    chcon_device_inode: Option<ChconDeviceAndINode>,
    dir_chcon_device_inode: ChconDeviceAndINode,
) -> bool {
    chcon_device_inode == Some(dir_chcon_device_inode)
}

fn chcon_root_dev_ino_warn(directory_name: &Path) {
    if directory_name.as_os_str() == "/" {
        ct_show_warning!(
            "It is dangerous to operate recursively on '/'. \
             Use --{} to override this failsafe.",
            opt_flags::preserve_root::NO_PRESERVE_ROOT,
        );
    } else {
        ct_show_warning!(
            "It is dangerous to operate recursively on {} (same as '/'). \
             Use --{} to override this failsafe.",
            directory_name.quote(),
            opt_flags::preserve_root::NO_PRESERVE_ROOT,
        );
    }
}

// 当fts_read返回FTS_DC表示目录循环时，这可能表示一个实际问题，也可能并非如此。
// 对于像chgrp这样的程序，如果在进行递归遍历过程中需要遍历符号链接，出现目录循环并不构成问题。
// 然而，当以"-P -R"选项调用时，这种情况应发出警告。
// fts_options参数记录了控制fts行为这一方面的选项，因此需要对此进行测试。
fn chcon_cycle_warning_required(fts_opts: c_int, entry: &fts::EntryRef) -> bool {
    // 当不解析任何符号链接，或者仅解析命令行上列出的符号链接且当前未处理命令行参数时，遇到循环则是严重的问题。
    ((fts_opts & fts_sys::FTS_PHYSICAL) != 0)
        && (((fts_opts & fts_sys::FTS_COMFOLLOW) == 0) || entry.level() != 0)
}

fn chcon_emit_cycle_warning(file_path: &Path) {
    ct_show_warning!(
        "Circular directory structure.\n\
This almost certainly means that you have a corrupted file system.\n\
NOTIFY YOUR SYSTEM MANAGER.\n\
The following directory is part of the cycle {}.",
        file_path.quote()
    );
}

#[derive(Debug)]
enum ChconSELinuxSecurityContext<'t> {
    File(SecurityContext<'t>),
    String(Option<CString>),
}

impl ChconSELinuxSecurityContext<'_> {
    fn to_c_string(&self) -> Result<Option<Cow<CStr>>> {
        match self {
            Self::File(context) => context
                .to_c_string()
                .map_err(|r| Error::from_selinux("SELinuxSecurityContext::to_c_string()", r)),

            Self::String(context) => Ok(context.as_deref().map(Cow::Borrowed)),
        }
    }
}

#[derive(Default)]
pub struct Chcon;
impl Tool for Chcon {
    fn name(&self) -> &'static str {
        "chcon"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        chcon_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::ffi::OsString;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_tool_implementation() {
        let tool = Chcon;

        // 测试 name 方法
        assert_eq!(tool.name(), "chcon");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("chcon"));

        // 测试 execute 方法
        let args = vec![OsString::from("chcon"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err()); // chcon需要参数，所以不带参数应该返回错误
    }

    #[test]
    fn test_ct_app_execution_help() {
        let command = ct_app();

        // 测试用例：有效输入 --help
        let args = vec![ctcore::ct_util_name(), "--help"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_ct_app_execution_help_invalid() {
        let command = ct_app();

        // 测试用例：有效输入 --help
        let args = vec![ctcore::ct_util_name(), "-H"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(
            executable.unwrap_err().kind(),
            ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn test_ct_app_execution_version() {
        let command = ct_app();

        // 测试用例：有效输入 --help
        let args = vec![ctcore::ct_util_name(), "--version"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_version_valid() {
        let command = ct_app();

        // 测试用例：有效输入 --help
        let args = vec![ctcore::ct_util_name(), "-V"];
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_dereference_true() {
        let command = ct_app();

        // 测试用例：有效输入 --dereference
        let args = vec![ctcore::ct_util_name(), "--dereference"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::dereference::DEREFERENCE));
    }

    #[test]
    fn test_ct_app_execution_dereference_false() {
        let command = ct_app();

        // 测试用例：有效输入 --no-dereference
        let args = vec![ctcore::ct_util_name(), "-h"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::dereference::NO_DEREFERENCE));
        assert!(!matches.get_flag(opt_flags::dereference::DEREFERENCE));
    }

    #[test]
    fn test_ct_app_execution_dereference_whole_false() {
        let command = ct_app();

        // 测试用例：有效输入 --no-dereference
        let args = vec![ctcore::ct_util_name(), "--no-dereference"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::dereference::NO_DEREFERENCE));
        assert!(!matches.get_flag(opt_flags::dereference::DEREFERENCE));
    }

    #[test]
    fn test_ct_app_execution_preserve_root_true() {
        let command = ct_app();

        // 测试用例：有效输入 --preserve-root
        let args = vec![ctcore::ct_util_name(), "--preserve-root"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::preserve_root::PRESERVE_ROOT));
    }

    #[test]
    fn test_ct_app_execution_preserve_root_false() {
        let command = ct_app();

        // 测试用例：有效输入 --no-preserve-root
        let args = vec![ctcore::ct_util_name(), "--no-preserve-root"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::preserve_root::NO_PRESERVE_ROOT));
        assert!(!matches.get_flag(opt_flags::preserve_root::PRESERVE_ROOT));
    }

    #[test]
    fn test_ct_app_execution_recursive() {
        let command = ct_app();

        // 测试用例：有效输入 --recursive
        let args = vec![ctcore::ct_util_name(), "-R"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::RECURSIVE));
    }

    #[test]
    fn test_ct_app_execution_recursive_whole() {
        let command = ct_app();

        // 测试用例：有效输入 --recursive
        let args = vec![ctcore::ct_util_name(), "--recursive"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::RECURSIVE));
    }

    // 对于布尔选项，例如 --verbose
    #[test]
    fn test_ct_app_execution_verbose() {
        let command = ct_app();

        // 测试用例：有效输入 --verbose
        let args = vec![ctcore::ct_util_name(), "-v"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::VERBOSE));
    }

    // 对于布尔选项，例如 --verbose
    #[test]
    fn test_ct_app_execution_verbose_whole() {
        let command = ct_app();

        // 测试用例：有效输入 --verbose
        let args = vec![ctcore::ct_util_name(), "--verbose"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::VERBOSE));
    }

    ///////////////////////////////////////////
    #[test]
    fn test_version_ctmain() {
        let args = [ctcore::ct_util_name(), "--version"];
        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_help_ctmain() {
        let args = [ctcore::ct_util_name(), "--help"];
        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_help_invalid_ctmain() {
        let args = [ctcore::ct_util_name(), "-H"];
        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_version_valid_ctmain() {
        let args = [ctcore::ct_util_name(), "-V"];
        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_dereference_true_ctmain() {
        let args = [ctcore::ct_util_name(), "--dereference"];
        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_dereference_false_ctmain() {
        let args = [ctcore::ct_util_name(), "-h"];
        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_dereference_whole_false_ctmain() {
        // 测试用例：有效输入 --no-dereference
        let args = [ctcore::ct_util_name(), "--no-dereference"];

        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_preserve_root_true_ctmain() {
        // 测试用例：有效输入 --preserve-root
        let args = [ctcore::ct_util_name(), "--preserve-root"];

        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_preserve_root_false_ctmain() {
        // 测试用例：有效输入 --no-preserve-root
        let args = [ctcore::ct_util_name(), "--no-preserve-root"];

        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_recursive_ctmain() {
        // 测试用例：有效输入 --recursive
        let args = [ctcore::ct_util_name(), "-R"];

        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_recursive_whole_ctmain() {
        // 测试用例：有效输入 --recursive
        let args = [ctcore::ct_util_name(), "--recursive"];

        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    // 对于布尔选项，例如 --verbose
    #[test]
    fn test_verbose_ctmain() {
        // 测试用例：有效输入 --verbose
        let args = [ctcore::ct_util_name(), "-v"];

        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    // 对于布尔选项，例如 --verbose
    #[test]
    fn test_verbose_whole_ctmain() {
        // 测试用例：有效输入 --verbose
        let args = [ctcore::ct_util_name(), "--verbose"];

        let result = chcon_main(args.iter().map(OsString::from));
        assert!(result.is_err());
    }

    #[test]
    fn test_chcon_ctmain() {
        // 创建文件并写入内容
        fn chcon_create_file_with_content(filename: &str, content: &str) -> io::Result<()> {
            let mut file = File::create(filename)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            Ok(())
        }

        // 删除指定文件
        fn chcon_delete_file(filename: &str) -> io::Result<()> {
            fs::remove_file(filename)?;
            Ok(())
        }

        let filename = "test_chcon_h_ctmain.txt";

        let content = "test_chcon_h_ctmain";

        // 创建文件并写入内容
        match chcon_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "system_u:object_r:httpd_sys_content_t",
            filename,
        ];

        let result = chcon_main(args.iter().map(OsString::from));

        // 删除文件
        match chcon_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }

        if let Err(e) = result {
            eprintln!("Error result: {e:?}");
        }
    }

    #[test]
    fn test_chcon_r_ctmain() {
        let dir_path = "test";
        let subdir_name = "subdirectory";
        let file_name = "test_chcon_h_ctmain.txt";

        // 创建二级目录
        let subdir_path = format!("{dir_path}/{subdir_name}");
        fs::create_dir_all(&subdir_path).expect("Failed to create directory");

        // 创建文件路径
        let file_path = format!("{subdir_path}/{file_name}");

        // 创建文件并写入内容
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"Hello, Rust!")
            .expect("Failed to write to file");
        println!("File '{file_path}' created successfully.");

        let args = [
            ctcore::ct_util_name(),
            "-R",
            "system_u:object_r:httpd_sys_content_t",
            dir_path,
        ];

        let result = chcon_main(args.iter().map(OsString::from));

        // 删除目录及其内容
        fs::remove_dir_all(dir_path).expect("Failed to delete directory");

        if let Err(e) = result {
            eprintln!("Error result: {e:?}");
        }
    }

    #[test]
    fn test_chcon_r_t_ctmain() {
        fn chcon_create_file_with_content(filename: &str, content: &str) -> io::Result<()> {
            let mut file = File::create(filename)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            Ok(())
        }

        // 删除指定文件
        fn chcon_delete_file(filename: &str) -> io::Result<()> {
            fs::remove_file(filename)?;
            Ok(())
        }

        let filename = "test_chcon_h_ctmain.txt";

        let content = "test_chcon_h_ctmain";

        // 创建文件并写入内容
        match chcon_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let args = [
            ctcore::ct_util_name(),
            "-R",
            "-t",
            "system_u:object_r:httpd_sys_content_t",
            filename,
        ];

        let result = chcon_main(args.iter().map(OsString::from));

        // 删除文件
        match chcon_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }

        if let Err(e) = result {
            eprintln!("Error result: {e:?}");
        }
    }
}
