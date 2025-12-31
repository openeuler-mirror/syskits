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

//! rmdir命令在Linux和其他类Unix系统中用于删除空目录, 如果目录非空，rmdir命令将会失败.

use clap::builder::ValueParser;
use clap::{crate_version, Arg, ArgAction, Command};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{set_ct_exit_code, strip_errno, CTResult};
use std::ffi::OsString;
use std::fs::{read_dir, remove_dir};
use std::io;
use std::path::Path;

use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show_error, ct_util_name};

const RMDIR_ABOUT: &str = ct_help_about!("rmdir.md");
const RMDIR_USAGE: &str = ct_help_usage!("rmdir.md");
pub mod rmdir_flags {
    pub const RMDIR_IGNORE_FAIL_NON_EMPTY: &str = "ignore-fail-on-non-empty";
    pub const RMDIR_PARENTS: &str = "parents";
    pub const RMDIR_VERBOSE: &str = "verbose";

    pub const RMDIR_ARG_DIRS: &str = "dirs";
}
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    rmdir_main(args)
}

pub fn rmdir_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let configs = RmdirConfigs {
        is_ignore: matches.get_flag(rmdir_flags::RMDIR_IGNORE_FAIL_NON_EMPTY),
        is_parents: matches.get_flag(rmdir_flags::RMDIR_PARENTS),
        is_verbose: matches.get_flag(rmdir_flags::RMDIR_VERBOSE),
    };

    for path in matches
        .get_many::<OsString>(rmdir_flags::RMDIR_ARG_DIRS)
        .unwrap_or_default()
        .map(Path::new)
    {
        if let Err(error) = rmdir_remove(path, configs) {
            let RmdirError { error, path } = error;

            if configs.is_ignore && rmdir_dir_not_empty(&error, path) {
                continue;
            }

            set_ct_exit_code(1);

            // 如果 `foo` 是一个指向目录的符号链接，那么 `rmdir foo/` 可能会给出 "不是目录" 错误。
            // 这很令人困惑，因为 `rm foo/` 会说 "是一个目录"。
            // 这在不同的系统中有所不同。有些不会报错。
            // Windows 允许对符号链接调用 RemoveDirectory，所以我们不需要在这里担心。
            // GNU rmdir 似乎会打印 "符号链接未跟随" 如果：
            // - 它有一个尾部斜杠
            // - 它是一个符号链接
            // - 它指向一个目录或是悬挂的
            #[cfg(unix)]
            {
                use std::ffi::OsStr;
                use std::os::unix::ffi::OsStrExt;

                fn points_to_directory(path: &Path) -> io::Result<bool> {
                    Ok(path.metadata()?.file_type().is_dir())
                }

                let bytes = path.as_os_str().as_bytes();
                if error.raw_os_error() == Some(libc::ENOTDIR) && bytes.ends_with(b"/") {
                    // 去除尾部斜杠，否则 .symlink_metadata() 会跟随符号链接
                    let no_slash: &Path = OsStr::from_bytes(&bytes[..bytes.len() - 1]).as_ref();
                    if no_slash.is_symlink() && points_to_directory(no_slash).unwrap_or(true) {
                        ct_show_error!(
                            "failed to remove {}: Symbolic link not followed",
                            path.quote()
                        );
                        continue;
                    }
                }
            }

            ct_show_error!("failed to remove {}: {}", path.quote(), strip_errno(&error));
        }
    }

    Ok(())
}

struct RmdirError<'a> {
    error: io::Error,
    path: &'a Path,
}

fn rmdir_remove(mut path: &Path, configs: RmdirConfigs) -> Result<(), RmdirError<'_>> {
    rmdir_remove_single(path, configs)?;
    if configs.is_parents {
        while let Some(new) = path.parent() {
            path = new;
            if path.as_os_str().is_empty() {
                break;
            }
            rmdir_remove_single(path, configs)?;
        }
    }
    Ok(())
}

fn rmdir_remove_single(path: &Path, configs: RmdirConfigs) -> Result<(), RmdirError<'_>> {
    if configs.is_verbose {
        println!("{}: removing directory, {}", ct_util_name(), path.quote());
    }
    remove_dir(path).map_err(|error| RmdirError { error, path })
}

// POSIX: https://pubs.opengroup.org/onlinepubs/009696799/functions/rmdir.html
#[cfg(not(windows))]
const RMDIR_NOT_EMPTY_CODES: &[i32] = &[libc::ENOTEMPTY, libc::EEXIST];

// 145 是 ERROR_DIR_NOT_EMPTY，通过实验确定。
#[cfg(windows)]
const RMDIR_NOT_EMPTY_CODES: &[i32] = &[145];

// 其他你可能会遇到的目录错误码，这些错误码表明目录存在但不为空。
// 这是来自 Linux man-pages 项目的 rmdir(2) 列出的错误码的一个子集。也许其他系统有额外的适用错误码？
#[cfg(not(windows))]
const RMDIR_PERHAPS_EMPTY_CODES: &[i32] = &[libc::EACCES, libc::EBUSY, libc::EPERM, libc::EROFS];

// 可能不完整，我找不到任何地方列出了 RemoveDirectory 可能的错误码。
#[cfg(windows)]
const RMDIR_PERHAPS_EMPTY_CODES: &[i32] = &[
    5, // ERROR_ACCESS_DENIED，通过实验确定。
];

fn rmdir_dir_not_empty(error: &io::Error, path: &Path) -> bool {
    if let Some(code) = error.raw_os_error() {
        if RMDIR_NOT_EMPTY_CODES.contains(&code) {
            return true;
        }
        // 如果使用了 --ignore-fail-on-non-empty，我们希望忽略所有由于目录不为空而导致的错误，
        // 即使错误是由于没有权限等原因造成的。因此我们进行额外的检查。
        if RMDIR_PERHAPS_EMPTY_CODES.contains(&code) {
            if let Ok(mut iterator) = read_dir(path) {
                return iterator.next().is_some();
            }
        }
    }
    false
}

#[derive(Clone, Copy, Debug)]
struct RmdirConfigs {
    is_ignore: bool,
    is_parents: bool,
    is_verbose: bool,
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = RMDIR_ABOUT;
    let usage_description = ct_format_usage(RMDIR_USAGE);
    let args = vec![
        Arg::new(rmdir_flags::RMDIR_IGNORE_FAIL_NON_EMPTY)
            .long(rmdir_flags::RMDIR_IGNORE_FAIL_NON_EMPTY)
            .help("ignore each failure that is solely because a directory is non-empty")
            .action(ArgAction::SetTrue),
        Arg::new(rmdir_flags::RMDIR_PARENTS)
            .short('p')
            .long(rmdir_flags::RMDIR_PARENTS)
            .help(
                "remove DIRECTORY and its ancestors; e.g.,
                  'rmdir -p a/b/c' is similar to rmdir a/b/c a/b a",
            )
            .action(ArgAction::SetTrue),
        Arg::new(rmdir_flags::RMDIR_VERBOSE)
            .short('v')
            .long(rmdir_flags::RMDIR_VERBOSE)
            .help("output a diagnostic for every directory processed")
            .action(ArgAction::SetTrue),
        Arg::new(rmdir_flags::RMDIR_ARG_DIRS)
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
}

