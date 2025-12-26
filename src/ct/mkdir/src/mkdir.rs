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

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::builder::ValueParser;
use clap::parser::ValuesRef;
use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};

#[cfg(not(windows))]
use ctcore::ct_error::FromIo;
use ctcore::ct_error::{CTResult, CtSimpleError};
#[cfg(not(windows))]
use ctcore::ct_mode;
use ctcore::{ct_display::Quotable, ct_fs::dir_strip_dot_for_creation};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_show_if_err};

const MKDIR_DEFAULT_PERM: u32 = 0o777;

const MKDIR_ABOUT: &str = ct_help_about!("mkdir.md");
const MKDIR_USAGE: &str = ct_help_usage!("mkdir.md");
const MKDIR_AFTER_HELP: &str = ct_help_section!("after help", "mkdir.md");

mod mkdir_flags {
    pub const MODE: &str = "mode";
    pub const PARENTS: &str = "parents";
    pub const VERBOSE: &str = "verbose";
    pub const DIRS: &str = "dirs";
}

#[cfg(windows)]
fn mkdir_get_mode(
    _arg_matches: &ArgMatches,
    _is_mode_had_minus_prefix: bool,
) -> Result<u32, String> {
    Ok(MKDIR_DEFAULT_PERM)
}

#[cfg(not(windows))]
fn mkdir_get_mode(arg_matches: &ArgMatches, is_mode_had_minus_prefix: bool) -> Result<u32, String> {
    // 未在 Windows 上测试
    let mut new_mode = MKDIR_DEFAULT_PERM;

    if let Some(m) = arg_matches.get_one::<String>(mkdir_flags::MODE) {
        for mode in m.split(',') {
            if mode.chars().any(|c| c.is_ascii_digit()) {
                new_mode = ct_mode::parse_numeric(new_mode, m, true)?;
            } else {
                let c_mode = match is_mode_had_minus_prefix {
                    true => {
                        // clap 解析完成，现在加回前缀
                        format!("-{mode}")
                    }
                    false => mode.to_string(),
                };
                new_mode = ct_mode::parse_symbolic(new_mode, &c_mode, ct_mode::get_umask(), true)?;
            }
        }
        Ok(new_mode)
    } else {
        // 如果未指定模式参数，则返回从 umask 派生的模式
        Ok(!ct_mode::get_umask() & 0o0777)
    }
}

#[cfg(windows)]
fn mkdir_strip_minus_from_mode(_args: &mut [String]) -> bool {
    false
}

#[cfg(not(windows))]
fn mkdir_strip_minus_from_mode(args: &mut [String]) -> bool {
    ct_mode::strip_minus_from_mode(args)
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    mkdir_main(args)
}

pub fn mkdir_main(args: impl ctcore::Args) -> CTResult<()> {
    let mut args = args.collect_lossy();

    // 在我们能用 clap（以及以前的 getopts）解析 'args' 之前，
    // 可能需要移除 MODE 前缀 '-'（例如 "chmod -x FILE"）。
    let is_mode_had_minus_prefix = mkdir_strip_minus_from_mode(&mut args);

    // Linux 特有的选项，未实现
    // opts.optflag("Z", "context", "set SELinux security context" +
    // " of each created directory to CTX"),
    let matches = ct_app().try_get_matches_from(args)?;

    let dirs = matches
        .get_many::<OsString>(mkdir_flags::DIRS)
        .unwrap_or_default();
    let is_verbose = matches.get_flag(mkdir_flags::VERBOSE);
    let is_recursive = matches.get_flag(mkdir_flags::PARENTS);

    match mkdir_get_mode(&matches, is_mode_had_minus_prefix) {
        Ok(mode) => mkdir_exec(dirs, is_recursive, mode, is_verbose),
        Err(f) => Err(CtSimpleError::new(1, f)),
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = MKDIR_ABOUT;
    let usage_description = ct_format_usage(MKDIR_USAGE);
    let args = vec![
        Arg::new(mkdir_flags::MODE)
            .short('m')
            .long(mkdir_flags::MODE)
            .help("set file mode (not implemented on windows)"),
        Arg::new(mkdir_flags::PARENTS)
            .short('p')
            .long(mkdir_flags::PARENTS)
            .help("make parent directories as needed")
            .action(ArgAction::SetTrue),
        Arg::new(mkdir_flags::VERBOSE)
            .short('v')
            .long(mkdir_flags::VERBOSE)
            .help("print a message for each printed directory")
            .action(ArgAction::SetTrue),
        Arg::new(mkdir_flags::DIRS)
            .action(ArgAction::Append)
            .num_args(1..)
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::DirPath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
        .after_help(MKDIR_AFTER_HELP)
}

/**
 * 创建新的目录列表
 */
fn mkdir_exec(
    dirs: ValuesRef<OsString>,
    is_recursive: bool,
    mode: u32,
    is_verbose: bool,
) -> CTResult<()> {
    for d in dirs {
        let p_buf = PathBuf::from(d);
        let p = p_buf.as_path();

        ct_show_if_err!(mkdir(p, is_recursive, mode, is_verbose));
    }
    Ok(())
}

/// 在给定的 `path` 处创建目录。
///
/// ## 选项
///
/// * `recursive` --- 创建 `path` 的父目录（如果不存在）。
/// * `mode` --- 目录的文件模式（在 windows 上未实现）。
/// * `verbose` --- 为每个创建的目录打印一条消息。
///
/// ## 尾随点
///
/// 为匹配 GNU 的行为，路径的最后一个目录是单个点（如 `some/path/to/.`）的情况会创建（并去除点）。
pub fn mkdir(path: &Path, is_recursive: bool, mode: u32, is_verbose: bool) -> CTResult<()> {
    // 特殊情况匹配 GNU 的行为：
    // mkdir -p foo/. 应该工作并只创建 foo/
    // std::fs::create_dir("foo/."); 在纯 Rust 中失败
    let path_buf = dir_strip_dot_for_creation(path);
    let path = path_buf.as_path();

    mkdir_create_dir(path, is_recursive, is_verbose, false)?;
    mkdir_chmod(path, mode)
}

#[cfg(unix)]
fn mkdir_chmod(path: &Path, mode: u32) -> CTResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::Permissions::from_mode(mode);

    std::fs::set_permissions(path, mode)
        .map_err_context(|| format!("cannot set permissions {}", path.quote()))
}

#[cfg(windows)]
fn mkdir_chmod(_path: &Path, _mode: u32) -> CTResult<()> {
    // 在 Windows 上，chmod 仅设置只读标志，该标志甚至不适用于目录
    Ok(())
}

// `is_parent` 参数在 windows 上不使用
#[allow(unused_variables)]
fn mkdir_create_dir(
    path: &Path,
    is_recursive: bool,
    is_verbose: bool,
    is_parent: bool,
) -> CTResult<()> {
    if path == Path::new("") {
        return Ok(());
    }

    if path.exists() && !is_recursive {
        let err_message = format!("{}: File exists", path.display());
        return Err(CtSimpleError::new(1, err_message));
    }

    if is_recursive {
        if let Some(p) = path.parent() {
            mkdir_create_dir(p, is_recursive, is_verbose, true)?;
        } else {
            CtSimpleError::new(1, "failed to create whole tree");
        }
    }

    if let Err(e) = std::fs::create_dir(path) {
        if path.is_dir() {
            Ok(())
        } else {
            Err(e.into())
        }
    } else {
        if is_verbose {
            println!(
                "{}: created directory {}",
                ctcore::ct_util_name(),
                path.quote()
            );
        }
        #[cfg(not(windows))]
        if is_parent {
            // 由 -p 创建的目录权限位设置为 '=rwx,u+wx'，
            // 即 umask 修改后的 'u+wx'
            mkdir_chmod(path, (!ct_mode::get_umask() & 0o0777) | 0o0300)?;
        }
        Ok(())
    }
}

